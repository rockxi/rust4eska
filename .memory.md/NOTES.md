## HTTPS / TLS (реализовано 2026-05-25)

### Архитектура
- Root CA + server TLS cert генерируются при первом старте мастера (`rcgen 0.12`)
- Хранятся в `vault_meta` Sled: `tls_ca_cert`, `tls_ca_key`, `tls_server_cert`, `tls_server_key`
- SANs: `master.r4a.local`, `*.master.r4a.local`, `*.r4a.local`, IP `10.42.0.1`
- HTTPS proxy на `<vpn_ip>:443`: `tokio-rustls 0.26` + `hyper-util` + `hyper`
- Host-based routing: `web.*` → 8081, `api.*` → 8080, всё остальное → 8000 (Pingora)
- `GET /api/ca-cert` — возвращает CA cert PEM без аутентификации

### Важные нюансы
- `rustls 0.23` требует `CryptoProvider::install_default()` при старте (`ring::default_provider().install_default()`)
- `axum-server 0.6` НЕ компилируется с нашим hyper 1.x — используем tokio-rustls напрямую
- Порт 443 привязан к `<vpn_ip>` (WireGuard IP, не 0.0.0.0) — доступен только через VPN/WireGuard
- DNS: `*.master.r4a.local` → `10.42.0.1`, `*.<node>.r4a.local` → node VPN IP
- CORS: добавлены `https://` варианты для `master.r4a.local`, `web.master.r4a.local`, `api.master.r4a.local`

### r4a-cli trust store
- macOS: `security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain /tmp/r4a-ca.crt`
- Debian: `/usr/local/share/ca-certificates/r4a-ca.crt` + `update-ca-certificates`
- Fedora: `/etc/pki/ca-trust/source/anchors/r4a-ca.crt` + `update-ca-trust extract`
- При `connect down` / Ctrl-C: удаляется из trust store
- `ConnectionState` добавлено поле `ca_cert_path: Option<String>`

### Доступ после connect up
- `https://web.master.r4a.local` — Web UI (→ port 8081)
- `https://api.master.r4a.local` — API (→ port 8080)
- `https://myapp.master.r4a.local` — user apps (→ port 8000 Pingora)
- Старые HTTP порты 8080/8081/8000 продолжают работать

---

## Security (реализовано 2026-05-21)

### Что изменилось в архитектуре безопасности

**Vault (C-1)**
- `master_salt` генерируется случайно (OsRng, 32 байта) при первом старте, хранится в `vault_meta["master_salt"]`.
- При старте: если расшифровка DEK новым ключом не работает — автоматический fallback на legacy-соль `b"r4a-master-salt-v1"`, затем перешифровка DEK и сохранение (одноразовая миграция).
- После миграции старый hardcoded ключ больше не используется.

**Привязка сокетов (C-2)**
- Агент API (8082): слушает на `resp.agent_vpn_ip`, а не `0.0.0.0`.
- Мастер API (8080): middleware `require_vpn_for_api` блокирует публичные IP для всех роутов кроме `/` и `/api/join`. Разрешены: `10.x.x.x`, `172.16-31.x.x`, `192.168.x.x`, loopback.
- Docker compose bridge (`172.20.0.0/16`) — разрешён, дашборд на `localhost:8081` работает.

**Sync whitelist (C-3)**
- `/api/store/sync` принимает только деревья: `tokens, bindings, policies, manifests, vault, vault_configs`.
- `vault_meta` и `core` (peers) — заблокированы.

**Сравнение секретов (H-1)**
- `constant_time_eq::constant_time_eq` везде где сравниваются `X-R4A-Secret` / токены.

**Роль при join (H-2)**
- `R4A_ALLOW_MASTER_JOIN=1` — только так можно добавить master-ноду.
- Без переменной все джойнеры получают роль `"agent"` независимо от запроса.
- **Сценарий multi-master**: временно выставить переменную на существующем мастере перед `r4a-server join-master`.

**IP пул (H-3)**
- `next_ip: Arc<Mutex<u16>>`, ограничение > 254 → 503 SERVICE_UNAVAILABLE.

**Секрет в сервисе (H-4)**
- `ServiceManager::enable` принимает `env_vars: &[(&str, &str)]`.
- Секреты пишутся в `/etc/r4a/<name>.env` (mode 0o600) через `EnvironmentFile=`.
- В cmdline (`ps aux`) секрет не виден.
- Все вызовы обновлены: r4a-server, r4a-agent, r4a-worker.

**Обновление бинарников (H-5)**
- Убран `Command::new(tmp_path).arg("--help")` — выполнение скачанного бинарника до замены.
- Верификация только по SHA256.

**CORS (H-6)**
- `AllowOrigin::predicate`: разрешены `http://10.42.*`, `http://master.local`, `http://master.r4a.local`, `http://localhost`, `http://127.0.0.1`.

---

## Vault (обновлено 2026-05-09)
- Поддержка множественных конфигураций Vault.
- Каждый конфиг имеет свой DEK.
- TUI: `[` / `]` для переключения конфигов, `Shift+C` для создания нового.
- Web UI: селектор конфигов и кнопка "New Config".
- Worker: поддержка `vault://config_id/key`.

---

## Инструменты тестирования
- Для проверки работоспособности кластера и API использовать `r4a-cli`.

## Web UI (реализовано 2026-05-09)
- **Backend**: Axum + rust-embed, порт `8081`.
- **Frontend**: React 19, TanStack Query, Tailwind CSS 4, Lucide React.
- **Динамический API**: фронтенд определяет адрес по `window.location.hostname`.

---

## Исправленные проблемы (2026-05-15)
- **401 на /api/manifests**: экстрактор `Auth` поддерживает и `X-R4A-Secret`, и `Bearer Token`.
- В `join_handler`: новые агенты автоматически получают права на `Resource::Manifests`.

---

## Окружение (prod)
- **asus** (rockxi-zenbook) — master, Ubuntu 24.04, kernel 6.17, Tailscale `100.97.158.58`
- **home** (DESKTOP-HIL871U) — agent, Ubuntu 24.04 WSL2, kernel 6.6, Tailscale `100.116.148.12`
- SSH: `ssh asus` / `ssh home`

## Компиляция и деплой
- Локально (macOS arm64) → `aarch64-unknown-linux-musl` (dev) или `x86_64-unknown-linux-musl` (prod)
- Бинарники статически слинкованы.
- `make dev-deploy` — сборка + `docker cp` + `docker restart`
- `make prod-deploy-all` — деплой на asus и home

---

## Worker: важные нюансы (2026-05-20)

### Label-изоляция контейнеров
- Агенты фильтруют по лейблу `r4a.node=<node_name>`.
- При 409: сначала `inspect_container` → если нет лейбла `r4a.node` — **не трогать** (ошибка), если есть — force remove и пересоздание с лейблом.
- В dev (shared Docker socket): при `node_selector = "all"` имя контейнера = `r4a-{name}-{node_name}`, при конкретной ноде — `r4a-{name}`. Это предотвращает конфликты между агентами на одном демоне.

### Image Pull
- `inspect_image` перед pull — если образ есть локально, pull пропускается. Критично для быстрого старта при повторных reconcile-циклах.

### Port Bindings
- Нужны и `HostConfig.PortBindings`, и `Config.ExposedPorts`.
- Формат: `ports: ["host_port:container_port"]`, например `"3333:80"`.

### Перезапуск агентов в docker compose
- `pkill` / `kill -9 1` внутри контейнера не работает.
- Правильно: `docker restart node-agent1`.

### make dev-deploy кэш
- Для принудительной пересборки: `touch crates/r4a-worker/src/lib.rs` перед `cargo build`.

### Перезапуск агентов в dev-deploy (исправлено 2026-05-25)
- `pkill -9 r4a-agent` внутри контейнера не работает — процесс убивается, но docker сразу перезапускает его со старым состоянием.
- Правильно: `docker restart node-agent1`. Теперь Makefile использует `docker restart`.

### Update flow (исправлено 2026-05-25)
- Агент при старте репортит `sha256_self()` со статусом "idle" → мастер знает текущую версию.
- Если при полле `self_checksum == master_checksum` → агент репортит "updated" (не молчит).
- Авто-сброс `update_pending` требует все агенты в статусе "Updated" + matching checksum.
- В статус-ответе: если checksum агента совпадает с мастером — показывается "updated" (не "idle"/"unknown").
- `R4A_SKIP_SIGNATURE_VERIFY=1` в compose.yaml для agent1/agent2 (без .sig файла в dev).

---

## Containers API (обновлено 2026-05-25)
- Агент слушает на `<vpn_ip>:8082` (не 0.0.0.0).
- Эндпоинты агента (Auth: `X-R4A-Secret`):
  - `GET /containers`
  - `GET /containers/:name/logs?tail=N`
  - `POST /containers/:name/restart`
  - `POST /containers/:name/stop`
  - `POST /containers/:name/start`
- Мастер проксирует через VPN IP:
  - `GET /api/nodes/:node/containers`
  - `GET /api/nodes/:node/containers/:name/logs?tail=N`
  - `POST /api/nodes/:node/containers/:name/restart`
  - `POST /api/nodes/:node/containers/:name/stop`
  - `POST /api/nodes/:node/containers/:name/start`
- Web UI: кнопка Stop/Start динамическая по `state` контейнера (`running` → Stop, иначе → Start)

---

## Manifests (State, обновлено 2026-05-20)
- Хранятся в Sled tree `"manifests"` (ключ = `app.name`).
- Миграция из старого blob-формата при старте.
- Агент: `GET /api/manifests?node=<name>`.

---

## RBAC
- Token / Policy / Binding система.
- `X-R4A-Secret` — только bootstrap эндпоинты (`/api/join`, `/api/metrics`, `/api/update/poll`...).
- User-facing эндпоинты — `Authorization: Bearer <token>`.

---

## Manifests: node_selector
- Обязательное поле — пустая строка не матчит ни одну ноду и ни `"all"`.
- Web UI: пустое поле блокирует Save + красная рамка.
- `"all"` в dev (shared Docker socket) — каждый агент получает манифест, имена контейнеров различаются суффиксом ноды.

---

## Connection (реализовано 2026-05-25)

### Архитектура
- Позволяет подключить машину к кластеру через WireGuard без регистрации как нода.
- Клиент получает VPN IP, настраивает WG туннель, но НЕ запускает воркеры/reconciler.
- Ingress (Pingora, порт 8000) доступен через туннель по IP `10.42.0.1`.

### Хранение
- Sled дерево `connections`: активные подключения (evict'ятся при disconnect или по таймауту 90s).
- Sled дерево `connection_labels`: `label → vpn_ip` — постоянный маппинг, IP не меняется при переподключении.

### API (Auth: Bearer token, Resource::Connections)
- `POST /api/connections` — создать подключение
- `DELETE /api/connections/:id` — отключиться
- `GET /api/connections` — список активных
- `POST /api/connections/:id/heartbeat` — продлить жизнь (каждые 30s)
- Фоновая задача: удалять connections где `last_seen > 90s`

### CLI (`r4a-cli`)
- `r4a-cli --master <url> --token <bearer> connect up [--label <name>]`
- `r4a-cli connect down`
- `r4a-cli connect status`
- `r4a-cli connect list`
- WG endpoint авто-деривируется из `--master` URL (host берётся из него).
- Heartbeat работает через `tokio::select!` в основном потоке.
- При Ctrl-C — DELETE на сервере + `wg-quick down wg0` + очистка `/etc/hosts`.
- Стейт сохраняется в `~/.r4a-connection.json`.

### r4a-client
- Добавлены методы: `connection_create`, `connection_delete`, `connection_heartbeat`, `connections_list`.
- `ApiClient::with_token(url, token)` — создать клиент с прямым Bearer токеном.

### DNS-сервер на мастере
- `run_dns_server(vpn_ip, store)` запускается в `start_server` — UDP на `10.42.0.1:53`.
- Разрешает `*.r4a.local`:
  - `master.r4a.local` → `10.42.0.1` (hardcoded)
  - `<node_name>.r4a.local` → VPN IP ноды (из `store.get("core", "peers")`)
  - `<label>.r4a.local` → VPN IP connection-клиента (из `store.get_label_ip(label)`)
  - Неизвестные `*.r4a.local` → NXDOMAIN
  - AAAA запросы для `*.r4a.local` → NOERROR пустой ответ (нет IPv6)
- Остальные домены → форвард на `8.8.8.8:53` (timeout 3s).
- Реализовано без внешних DNS-крейтов (raw UDP + ручной парсинг/сборка DNS пакетов).

### DNS на macOS (схема r4a.local)
- При `connect up`:
  - `/etc/hosts`: `10.42.0.1 master.r4a.local` и `<vpn_ip> <label>.r4a.local` (fallback)
  - `/etc/resolver/r4a.local`: `nameserver 10.42.0.1` — macOS направляет все `*.r4a.local` на наш DNS
- Динамические имена нод (`agent1.r4a.local` и т.д.) теперь разрешаются через DNS (не через /etc/hosts).
- При `connect down` / Ctrl-C — удаляются `/etc/hosts` записи и `/etc/resolver/r4a.local`.
- Стейт хранит `added_hosts: Vec<String>` и `resolver_domain: Option<String>` в `~/.r4a-connection.json`.
- Web UI: `http://master.r4a.local:8081`, Ingress: `http://master.r4a.local:8000`.
- Браузер: явно `http://` (не https). Firefox кэширует HSTS — очистить данные сайта.

### compose.yaml
- Добавлен проброс порта `51820:51820/udp` для WireGuard.

### Нюансы
- IP всегда один и тот же для одного label — хранится в `connection_labels`.
- Без label — IP динамический (из пула `next_ip`).
- WG endpoint: при `--master http://localhost:8080` → `localhost:51820` (авто).
- `r4a-cli --token` / `R4A_TOKEN` — аутентификация по Bearer токену (альтернатива `--secret`).
- Web UI через VPN: `http://master.r4a.local:8081` (React app), API на `:8080`, Ingress на `:8000`.
- CORS настроен на `master.r4a.local` — Web UI работает при подключении через `connect up`.

---

## Инструкция по разработке
1. `make dev-up`
2. Меняем код → `make dev-deploy`
3. TUI: `docker exec -it node-agent1 R4A_SECRET=test_secret_for_cluster_123 r4a-tui`
4. Логи: `docker compose logs agent1 agent2 -f`

## 2026-07-13: admin_secret отделён от cluster_secret
- `/api/tokens/exchange` теперь требует admin-секрет (`R4A_ADMIN_SECRET` / генерируется в identity.json), cluster-секрет больше не даёт admin-токен.
- Проверено в docker compose: cluster-секрет → 401, admin-секрет → 200, агенты джойнятся как раньше.
- ВАЖНО для prod-деплоя: вход в web UI теперь по admin-секрету (смотреть в ~/.r4a-server/identity.json на мастере).
- Из код-ревью остались нерешёнными: delete не реплицируется между мастерами; RBAC `can()` матчит resource_names как префиксы (starts_with); CORS-предикат обходится префиксом (10.42.evil.com); мёртвый поиск existing_token_id в join_handler; next_ip не учитывает connections после рестарта.
