# NOTES

## Окружение

- **asus** (rockxi-zenbook) — master-нода, Ubuntu 24.04, kernel 6.17, Tailscale IP 100.97.158.58
- **home** (DESKTOP-HIL871U) — agent-нода, Ubuntu 24.04 WSL2, kernel 6.6, Tailscale IP 100.116.148.12
- SSH через Tailscale: `ssh asus` / `ssh home` (конфиг в ~/.ssh/config)
- **Роли**: asus — единственный Master, home — Agent (multi-master конфигурация для home была отменена).

## Компиляция и деплой

- Собираем локально (macOS arm64) в таргет `x86_64-unknown-linux-musl`
- Линкер: `x86_64-linux-musl-gcc` (homebrew)
- Конфиг в `.cargo/config.toml`
- Бинарники статически слинкованы — нет зависимостей на хостах
- **Деплой всегда в `/usr/local/bin/`**: Сборка локально -> `scp` на хост -> регистрация через `r4a-server service enable` / `r4a-agent service enable`.
- **Перезапуск**: При деплое вызывается `sudo systemctl restart <service>`.

## WireGuard

- VPN подсеть: `10.42.0.0/24`
- master: `10.42.0.1/24`, listen port 51820
- agents: `10.42.0.2+/32`, assigned динамически при join
- На home (WSL2) WireGuard kernel module есть, всё работает
- Для первого join агент использует Tailscale IP мастера: `http://100.97.158.58:8080`

## Порты r4a-server

- `0.0.0.0:8080` — основной порт сервера (API + git).
- `0.0.0.0:8000` — автоматический Ingress на базе Pingora.
- На asus основной порт 80 занят nginx, поэтому Ingress перенесен на 8000.
- TUI и агент ходят на `http://master.local:8080`, но `master.local` резолвится во все VPN IP мастеров.

## Статус и Здоровье нод (Healthchecks)
- Мастер отслеживает `last_seen` для каждой ноды на основе прилетающих метрик.
- В TUI добавлена колонка **Status**:
    - **ONLINE** (зеленый): метрики получены менее 20 секунд назад.
    - **OFFLINE** (красный): задержка более 20 секунд.
- Ноды, не подававшие признаков жизни более 10 минут, скрываются из списка API.

## Реализация манифестов (2026-04-29)
- Создан крейт `r4a-core`: содержит общие модели `Manifest`, `NodeInfo`, `PeerInfo`, `JoinRequest` и др.
- **Мастер**: Каждые 10 секунд сканирует `manifests.git` (bare repo), парсит `.toml` файлы и сохраняет в Sled БД (дерево `manifests`).
- **Агент**: Каждые 30 секунд запрашивает манифесты через `GET /api/manifests?node=<имя_ноды>`.
- **Worker**: Крейт `r4a-worker` (используется агентом) управляет Docker (через `bollard`) и Systemd.
    - Автоматически останавливает и удаляет "бесхозные" контейнеры (с префиксом `r4a-`), которых нет в текущих манифестах.
    - Поддерживает проброс портов (`ports = ["host:container"]`) и переменные окружения.

## Ингресс (Pingora)
- Реализован в `crates/r4a-ingress`.
- Слушает на `0.0.0.0:8000`.
- Маршрутизация: запросы `app-name.master.local` или по заголовку `Host` проксируются на VPN IP соответствующего агента.

## Нюансы сборки и деплоя
- **Pingora** требует `cmake` и фиксацию версии `sfv = "0.9.3"` в Cargo.lock.
- **Имя ноды**: При установке агента крайне важно указывать имя, если оно отличается от hostname:
  `sudo r4a-agent service enable --master http://... --name home`

## Проблемы которые встретились
- **Runtime within Runtime**: Pingora нельзя запускать через `tokio::spawn`, так как у неё свой runtime. Решено через `std::thread::spawn`.
- **BindError**: При рестарте порт 8000 может быть занят зависшим процессом. Решено через `pkill -9 r4a-server` в Makefile и биндинг на `0.0.0.0`.
- **Orphan Cleanup**: Агенты удаляли контейнеры друг друга из-за несовпадения имен нод. Решено фиксацией `--name home` в сервисе.


## Git-экран в TUI (реализовано 2026-04-28)

### Суть
Новая вкладка "Git" в TUI показывает список bare-репозиториев с мастера.

### API
- `GET /api/git/repos` — листает `~/.r4a-server/git/`, ищет папки с `HEAD` (bare-репозитории)
  - Возвращает: `[{name, clone_url}]`
  - clone_url формат: `http://10.42.0.1:8080/git/<name>`

### TUI
- `Screen::Git` — вторая вкладка (между Dashboard и RBAC)
- Отображает: имя репо (зелёным) + `git clone <url>` (серым)
- Обновляется каждые 2 сек пока вкладка активна

### Нюансы
- При первом запуске новой версии сервера старый процесс надо убивать явно через `sudo kill <pid>` — `pkill` без sudo не может завершить root-процесс
- Текущий список репозиториев: `manifests.git`

## Создание репозиториев через TUI (реализовано 2026-04-28)

### API
- `POST /api/git/repos` — создаёт новый bare-репозиторий
  - Принимает: `{"name": "repo-name"}` (расширение `.git` добавляется автоматически)
  - Возвращает: `{name, clone_url}`
  - Ошибки: 400 (пустое имя / `/` / `..`), 409 (уже существует), 500

### TUI
- На экране Git: клавиша `n` открывает строку ввода имени репозитория
- `Enter` — создать, `Esc` — отмена, `Backspace` — удалить символ
- После создания список автоматически обновляется, показывается сообщение

## Каскадное обновление агентов (реализовано 2026-04-28)

### Суть
Мастер хранит актуальный бинарник `r4a-agent` в `/usr/local/bin/r4a-agent`.
Агенты каждые 30 сек поллят мастер и при несовпадении SHA256 скачивают и заменяют себя.
TUI-экран Update позволяет инициировать процесс.

### Новые API-эндпоинты (r4a-server)
- `GET  /api/agent-binary`      — отдаёт бинарник агента (application/octet-stream)
- `GET  /api/agent-checksum`    — `{"checksum": "<sha256>"}`
- `POST /api/update/test`       — проверяет наличие и SHA256 бинарника, возвращает `{ok, checksum, message}`
- `POST /api/update/trigger`    — выставляет `update_pending = true` в AppState
- `GET  /api/update/poll`       — агенты опрашивают: `{update_pending, checksum}`
- `POST /api/update/report`     — агент сообщает статус `{agent_vpn_ip, checksum, status}`
- `GET  /api/update/status`     — TUI: checksum мастера + статус всех агентов

### Логика r4a-agent (auto-update loop)
- Отдельный `tokio::spawn` стартует после join
- Каждые 30 сек: GET /api/update/poll
- Если `update_pending = true` И SHA256 отличается от собственного:
  1. Скачивает бинарник в `/tmp/r4a-agent.new`
  2. Проверяет SHA256 совпадение
  3. `chmod 755` + `mv` в `/usr/local/bin/r4a-agent`
  4. POST /api/update/report со статусом "updated"
  5. `std::process::exit(0)` — systemd/перезапуск подхватит
- SHA256 себя считается через `std::env::current_exe()` + чтение файла

### TUI-экран Update
- Новая вкладка "Update" (пятая, после Observability)
- Отображает: checksum мастера, per-agent IP + checksum + статус (цвет: green=ok, yellow=updating, red=fail)
- Клавиши:
  - `t` — POST /api/update/test (проверить бинарник на мастере, показать результат)
  - `u` — POST /api/update/trigger (запустить обновление всех агентов)
- Статус обновляется каждые 2 сек вместе с основным refresh

### Зависимости
- `sha2 = "0.10"` добавлена в workspace и в r4a-server, r4a-agent

### Нюансы
- `update_pending` не сбрасывается автоматически после обновления — сервер не имеет стейта о том, все ли агенты обновились. Сброс в будущем можно добавить когда все агенты отрепортят "updated".
- Агент завершается сам через exit(0) после обновления — нужен внешний supervisor (systemd/перезапуск вручную) для перезапуска с новым бинарником.
- На asus r4a-server запускается через `sudo nohup` (нужен root для WireGuard).

## Улучшение связности и TUI (реализовано 2026-04-28)

### Тестирование в Docker
- Создан `compose.yaml` для имитации кластера.
- **Мастер**: доступен внутри сети Docker как `master:8080`.
- **Секрет**: задается через `R4A_SECRET=test_secret_for_cluster_123`.
- **Ресурсы**: ограничены 1 vCPU и 1GB RAM на ноду.
- **WireGuard**: требует `NET_ADMIN` и `/dev/net/tun` на хосте. В macOS Docker Desktop это работает через виртуализацию.
- **Docker**: агенты используют сокет хоста `/var/run/docker.sock` для запуска нагрузок.

## DNS и master.local
- `master.local` теперь резолвится во **все** доступные master-ноды.
- `r4a-server` при старте и каждые 10 секунд обновляет свой `/etc/hosts`, добавляя туда IP всех известных мастеров (включая себя).
- Это позволяет использовать `http://master.local:8080` как универсальный эндпоинт для API.

### r4a-tui
- Дефолтный URL мастера изменен на `http://master.local:8080`.
- Добавлена поддержка переменной окружения `R4A_MASTER` (например, `R4A_MASTER=http://100.97.158.58:8080 r4a-tui`).
- Исправлена ошибка сборки: в `clap` добавлен feature `env`.

### Деплой
- `Makefile` обновлен: теперь `deploy-all` включает `deploy-master`, `deploy-agent` и `deploy-tui`.
- Все деплой-цели автоматически перезапускают системные сервисы.
- `deploy-tui` копирует TUI сразу на все ноды кластера.

## Структура проекта

```
r4a/
├── Cargo.toml                  # workspace
├── .cargo/config.toml          # musl линкер
├── crates/
│   ├── r4a-vpn/                # WireGuard + /etc/hosts
│   ├── r4a-store/              # Raft-lite + Sled (репликация)
│   └── r4a-git-registry/       # git init + git http-backend handler
└── binaries/
    ├── r4a-server/             # master: wg + axum HTTP + metrics + git + store
    ├── r4a-agent/              # agent: join + wg + dns + metrics reporter
    └── r4a-tui/                # TUI: dashboard (nodes/CPU/RAM/VRAM), заглушки
```