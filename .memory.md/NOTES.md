# NOTES

## Окружение

- **asus** (rockxi-zenbook) — master-нода, Ubuntu 24.04, kernel 6.17, Tailscale IP 100.97.158.58
- **home** (DESKTOP-HIL871U) — agent-нода, Ubuntu 24.04 WSL2, kernel 6.6, Tailscale IP 100.116.148.12
- SSH через Tailscale: `ssh asus` / `ssh home` (конфиг в ~/.ssh/config)

## Компиляция и деплой

- Собираем локально (macOS arm64) в таргет `x86_64-unknown-linux-musl`
- Линкер: `x86_64-linux-musl-gcc` (homebrew)
- Конфиг в `.cargo/config.toml`
- Бинарники статически слинкованы — нет зависимостей на хостах
- **Деплой всегда в `/usr/local/bin/`**: `scp bin host:/tmp/bin && ssh host "sudo mv /tmp/bin /usr/local/bin/bin"`

## WireGuard

- VPN подсеть: `10.42.0.0/24`
- master: `10.42.0.1/24`, listen port 51820
- agents: `10.42.0.2+/32`, assigned динамически при join
- На home (WSL2) WireGuard kernel module есть, всё работает
- Для первого join агент использует Tailscale IP мастера: `http://100.97.158.58:8080`

## Порты r4a-server

- `0.0.0.0:8080` — единственный порт сервера (API + git)
- Порт 80 **не занимаем** — на asus работает nginx, конфликт недопустим
- TUI и агент ходят на `http://10.42.0.1:8080`

## API endpoints (r4a-server)

- `GET /` — healthcheck
- `POST /api/join` — агент регистрируется, получает VPN IP; принимает `name` (hostname по умолчанию)
- `GET /api/nodes` — список нод с метриками (CPU, RAM, VRAM)
- `POST /api/metrics` — агент шлёт CPU/RAM/VRAM каждые 5 сек
- `ANY /git/*` — Git HTTP Smart Protocol (через git http-backend CGI)

## Персистентность (важно!)

- `~/.r4a-server/identity.json` — keypair мастера. Генерируется один раз, при рестарте загружается.
  Без этого агенты теряют соединение при каждом рестарте мастера.
- `~/.r4a-server/peers.json` — список пиров (pub_key, vpn_ip, name). При рестарте мастера
  WireGuard поднимается сразу со всеми сохранёнными пирами.
- При повторном join (агент перезапустился) мастер распознаёт pub_key и возвращает тот же IP.

## Git-хранилище манифестов

- `~/.r4a-server/git/manifests.git` — bare-репозиторий, создаётся автоматически при `r4a-server init`
- Доступен по: `http://10.42.0.1:8080/git/manifests.git`
- Требует `git` на хосте (используется `git http-backend`)
- Крейт: `crates/r4a-git-registry`

## Метрики

- CPU/RAM через `sysinfo`
- VRAM через `nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader,nounits`
- Если nvidia-smi нет — поля `null` / показывается `—` в TUI
- asus: GPU 2048 MB VRAM; home: GPU 16311 MB VRAM

## Имена нод

- По умолчанию — hostname системы (через `System::host_name()`)
- Можно переопределить: `r4a-agent connect --master ... --name my-node`

## Проблемы которые встретились

- На asus nginx занимает 0.0.0.0:80 — r4a-server не биндится на 80, только на 8080.
- При старой версии (без identity.json) каждый рестарт генерировал новый keypair — агенты теряли соединение.
- peers.json накапливает стейл-записи если агент переподключается с новым keypair (каждый connect генерирует новый ключ). Решение в будущем: персистировать keypair агента.
- TUI нельзя настраивать на master.local без предварительной записи в /etc/hosts — используем 10.42.0.1:8080 напрямую.

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

## Структура проекта

```
r4a/
├── Cargo.toml                  # workspace
├── .cargo/config.toml          # musl линкер
├── crates/
│   ├── r4a-vpn/                # WireGuard + /etc/hosts
│   └── r4a-git-registry/       # git init + git http-backend handler
└── binaries/
    ├── r4a-server/             # master: wg + axum HTTP + metrics + git
    ├── r4a-agent/              # agent: join + wg + dns + metrics reporter
    └── r4a-tui/                # TUI: dashboard (nodes/CPU/RAM/VRAM), заглушки
```
