# rust4eska (r4a)

[English version](README.md)

Лёгкая, самодостаточная система управления кластером на Rust. Один мастер, любое число агентов, связанных встроенной WireGuard mesh-VPN — без Nginx, внешнего VPN, внешней БД и Docker Registry. Всё поставляется статическими бинарниками.

## Возможности

- **Встроенный VPN** — автоматический WireGuard-mesh (`10.42.0.0/16`) между мастером и агентами; прямые P2P-линки между агентами, когда NAT позволяет, и автоматический relay через мастер, когда нет.
- **Встроенный DNS** — мастер раздаёт имена `*.r4a.local` по VPN, без правки `/etc/hosts`.
- **Edge-роутинг** — ingress на Pingora маршрутизирует `<app>.<node>.r4a.local` в контейнеры на любой ноде.
- **Workloads** — декларативные TOML-манифесты превращаются в Docker-контейнеры на агентах.
- **Git и Registry** — встроенный bare-git-хостинг и OCI-реестр.
- **Vault и RBAC** — шифрованное хранилище секретов (`vault://`-ссылки в манифестах), токены и политики.
- **Дашборды** — терминальный UI (`r4a-tui`) и веб-интерфейс на React (`r4a-web`).
- **Обновление кластера** — одна клавиша в TUI обновляет подписанные бинарники по всему кластеру.

## Установка бинарников

Каждая роль ниже начинается со скачивания нужных ей бинарников из [GitHub Releases](https://github.com/rockxi/rust4eska/releases), для Linux (x86_64, статическая musl-сборка) и macOS (x86_64 / arm64):

```bash
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  T=x86_64-linux-musl ;;
  Darwin-x86_64) T=x86_64-macos ;;
  Darwin-arm64)  T=aarch64-macos ;;
  *) echo "неподдерживаемая платформа: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
for bin in r4a-server r4a-agent r4a-cli r4a-tui; do
  curl -fL -o "$bin" "https://github.com/rockxi/rust4eska/releases/latest/download/${bin}-${T}"
  chmod +x "$bin"
done
sudo install -m 755 r4a-server r4a-agent r4a-cli r4a-tui /usr/local/bin/
```

Не обязательно тащить все четыре бинарника на каждую машину — смотрите, какие именно нужны каждой роли ниже.

Общие требования (Linux и macOS): поддержка WireGuard (любое современное ядро Linux, либо `wireguard-go` на macOS) + `wireguard-tools`, `iproute2`, `iptables` на Linux; root-доступ (настройка VPN-интерфейса); Docker — только на нодах, которые будут запускать workloads.

Установка пакетов:

```bash
# Ubuntu/Debian
sudo apt update && sudo apt install -y wireguard-tools iproute2 iptables

# macOS (Homebrew)
brew install wireguard-tools wireguard-go
```

## Настройка мастер-ноды

Мастер запускает `r4a-server`: он держит WireGuard mesh (`10.42.0.0/16`), control API, DNS для `*.r4a.local`, ingress и состояние кластера. Нужен `r4a-server` (плюс `r4a-cli` / `r4a-tui` для управления) из раздела [Установка бинарников](#установка-бинарников).

Снаружи мастера должны быть доступны порты:

| Порт | Протокол | Назначение |
|------|----------|------------|
| `51820` | UDP | WireGuard (обязательно открыть / пробросить — критично) |
| `3501` | TCP | Control API (не из VPN доступны только `/` и `/api/join`) |

Если мастер за домашним роутером — пробросьте `51820/udp` (и `3501/tcp`) на него. На Linux также разрешите эти порты в `iptables`/`ufw`/`firewalld`, на macOS — в Системных настройках → Firewall, если он включён.

Установка в одну команду (скачивает бинарники, ставит зависимости WireGuard через apt/brew, генерит секреты, запускает как сервис):

```bash
curl -fsSL https://raw.githubusercontent.com/rockxi/rust4eska/main/scripts/install-server.sh | sudo bash
```

Секреты будут выведены один раз в конце — сохраните их. Либо по шагам, с полным контролем:

```bash
sudo -E r4a-server install
```

Либо полностью вручную:

```bash
export R4A_SECRET=$(openssl rand -hex 16)         # секрет кластера — передайте агентам/клиентам
export R4A_ADMIN_SECRET=$(openssl rand -hex 16)   # админ-секрет — для управления через CLI/TUI/Web UI (держите при себе)
echo "cluster secret: $R4A_SECRET"; echo "admin secret: $R4A_ADMIN_SECRET"

# Если мастер за NAT — укажите публичный endpoint:
export R4A_PUBLIC_ENDPOINT=<ваш-публичный-ip>:51820

sudo -E r4a-server init          # в форграунде, удобно для первого теста
# либо как systemd (Linux) / launchd (macOS) сервис, с теми же переменными окружения:
sudo -E r4a-server service enable
```

Мастер получает VPN-IP `10.42.0.1`. Состояние хранится в `~/.r4a-server/`.

Проверка:

```bash
r4a-cli --master http://10.42.0.1:3501 --secret <админ-секрет> nodes list
R4A_MASTER=http://10.42.0.1:3501 R4A_SECRET=<админ-секрет> r4a-tui   # дашборд; колонка "P2P" показывает прямые линки
```

## Настройка агент-ноды

Агент подключается к уже работающему мастеру и может запускать workloads (Docker-контейнеры из TOML-манифестов). Нужен `r4a-agent` (плюс Docker) из раздела [Установка бинарников](#установка-бинарников) и уже запущенный мастер.

Установка в одну команду (скачивает бинарник, ставит зависимости WireGuard через apt/brew, подключается к мастеру как сервис):

```bash
curl -fsSL https://raw.githubusercontent.com/rockxi/rust4eska/main/scripts/install-agent.sh | sudo bash -s -- \
  --master http://<публичный-ip-мастера>:3501 --secret <секрет-кластера> --name friend1
```

Либо полностью вручную:

```bash
sudo r4a-agent connect \
  --master http://<публичный-ip-мастера>:3501 \
  --secret <секрет-кластера> \
  --name friend1
# постоянно (systemd на Linux / launchd на macOS):
sudo r4a-agent service enable --master http://<публичный-ip-мастера>:3501 --secret <секрет> --name friend1
```

Проверка (с агента или с любой машины, уже подключённой к VPN):

```bash
ping 10.42.0.1                      # мастер через VPN
r4a-cli --master http://10.42.0.1:3501 --secret <админ-секрет> nodes list   # агент должен появиться в списке
```

## Подключение только как VPN-клиент

Используйте это, если нужен просто доступ к существующему кластеру (пинговать ноды, ходить на `*.r4a.local`, пользоваться API/TUI) без запуска workloads. Нужен только `r4a-cli` (добавьте `r4a-tui` для дашборда) из раздела [Установка бинарников](#установка-бинарников) и уже запущенный мастер.

```bash
export R4A_MASTER=http://<публичный-ip-мастера>:3501
export R4A_SECRET=<секрет-кластера>
sudo -E r4a-cli connect up --label my-laptop
r4a-cli connect status
```

Проверка:

```bash
ping 10.42.0.1                      # мастер через VPN
r4a-cli --master http://10.42.0.1:3501 --secret <админ-секрет> nodes list
```

Отключение: `r4a-cli connect down` (или `r4a-cli connect cleanup`, если после неудачного отключения что-то осталось).

Если что-то не работает — см. [Диагностику](#диагностика).

## Деплой фронтенда и workload'ов

### Web UI

Запускается на мастере (Linux или macOS):

```bash
r4a-web --port 3502
```

Откройте `http://10.42.0.1:3502` и войдите по админ-секрету. Здесь можно создавать/редактировать манифесты workload'ов визуально, управлять нодами, секретами и RBAC.

### Деплой workload'а через API

Workload'ы описываются TOML-манифестами (пример — `postgres.toml`) и разворачиваются в Docker-контейнеры на агентах.

```bash
# обменять админ-секрет на bearer-токен и отправить манифест:
TOKEN=$(curl -s -X POST http://10.42.0.1:3501/api/tokens/exchange \
  -H "X-R4A-Secret: <админ-секрет>" | jq -r .id)
curl -X POST http://10.42.0.1:3501/api/manifests \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" -d @manifest.json
r4a-cli manifests list        # посмотреть задеплоенные манифесты
```

Приложение станет доступно по адресу `<app>.<node>.r4a.local` через встроенный ingress и DNS (только внутри VPN).

## Dev-кластер (Docker Compose)

Локально поднимает 1 мастер + 2 агента:

```bash
make dev-up        # собрать и запустить
make dev-deploy    # пересобрать и залить бинарники в работающие контейнеры
make dev-down
```

- Web UI: `http://localhost:3502` — вход по админ-секрету `test_admin_secret_456`
- API: `http://localhost:3501`, ingress: `http://localhost:3500`
- TUI: `R4A_MASTER=http://localhost:3501 R4A_SECRET=test_admin_secret_456 r4a-tui` или `docker exec -it node-master r4a-tui`

Требуется: Rust stable, Node.js (фронтенд), Docker, musl-таргет (`rustup target add x86_64-unknown-linux-musl`).

## Обновление кластера

1. Откройте `r4a-tui` → вкладка **Update** → клавиша `u` — обновление по всему кластеру.
2. Бинарники проверяются ed25519-подписью. Самособранные бинарники официальную подпись не пройдут — поставьте `R4A_SKIP_SIGNATURE_VERIFY=1` на агентах (только dev/test).

## Справочник портов и переменных

| Порт | Где | Назначение |
|------|-----|------------|
| 51820/udp | мастер и агенты | WireGuard |
| 3501 | мастер | Control API (вне VPN — только `/api/join`) |
| 3500 | мастер | Ingress (Pingora) |
| 3502 | мастер | Web UI (`r4a-web`) |
| 443 | VPN-IP мастера | HTTPS-прокси (только VPN) |
| 53 | VPN-IP мастера | DNS для `*.r4a.local` (только VPN) |
| 8082 | VPN-IP агента | Agent API (только VPN) |

| Переменная | Назначение |
|------------|------------|
| `R4A_SECRET` | Секрет кластера (нужен для входа; на мастере генерируется автоматически, если не задан — см. `~/.r4a-server/identity.json`) |
| `R4A_ADMIN_SECRET` | Админ-секрет — обменивается на управляющий токен (CLI/TUI/Web UI) |
| `R4A_PUBLIC_ENDPOINT` | Публично доступный `host:51820` — обязателен за NAT (мастер, опционально агенты) |
| `R4A_MASTER` | URL API мастера для CLI/TUI (по умолчанию `http://master.r4a.local:3501`) |
| `R4A_TOKEN` | RBAC bearer-токен (альтернатива секрету) |
| `R4A_SKIP_SIGNATURE_VERIFY` | `1` = пропустить проверку подписи релиза (только dev) |

## Диагностика

- **Агент подключился, но ping через VPN не идёт** — `51820/udp` недоступен снаружи. Проверьте проброс порта на роутере мастера и задайте `R4A_PUBLIC_ENDPOINT` до запуска мастера.
- **В колонке P2P relay вместо direct** — оба пира за строгими NAT; трафик автоматически идёт через мастер. Прямой P2P между двумя разными NAT пока ненадёжен ([известное ограничение](#известные-ограничения-mvp)).
- **`*.r4a.local` не резолвится** — DNS работает только внутри VPN (`10.42.0.1:53`). Используйте VPN-IP напрямую (`http://10.42.0.1:3501`), если ОС не подхватила резолвер.
- **API отвечает 403 снаружи** — так задумано: всё, кроме `/` и `/api/join`, доступно только из VPN.
- **Остатки интерфейсов/DNS после неудачного отключения** — `r4a-cli connect cleanup`.

## Известные ограничения (MVP)

- Прямой P2P между двумя агентами, каждый из которых за своим NAT, может не устанавливаться — используется relay через мастер.
- Синхронизация мульти-мастера — HTTP push, не Raft-консенсус.
- Ключ подписи релизов — заглушка; проверка подписи важна только для встроенного механизма обновления.

## Структура проекта

- `binaries/` — `r4a-server` (мастер), `r4a-agent`, `r4a-cli`, `r4a-tui`, `r4a-web` (встроенная React SPA)
- `crates/` — `r4a-core` (типы/крипто), `r4a-vpn` (WireGuard+DNS), `r4a-store` (Sled+sync+vault+RBAC), `r4a-ingress` (Pingora), `r4a-git-registry`, `r4a-worker` (Docker-реконсилер), `r4a-service`, `r4a-telemetry`, `r4a-client`

## Лицензия

MIT / Apache-2.0
