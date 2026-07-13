# TODO

- [x] **Feature: HTTPS через самоподписанный CA (2026-05-25)**
    - [x] rcgen: генерация root CA + server cert (SANs: `master.r4a.local`, `*.master.r4a.local`, `*.r4a.local`, IP `10.42.0.1`)
    - [x] Хранение CA/cert в `vault_meta` Sled дереве
    - [x] `GET /api/ca-cert` endpoint (без auth, VPN-only)
    - [x] HTTPS proxy на `<vpn_ip>:443` (tokio-rustls + hyper-util)
    - [x] Host-based routing: `web.*` → 8081, `api.*` → 8080, остальное → 8000 (Pingora)
    - [x] DNS: `*.master.r4a.local` → 10.42.0.1, `*.<node>.r4a.local` → node VPN IP
    - [x] CORS: добавлены https:// варианты origin
    - [x] rustls 0.23: `ring::default_provider().install_default()` при старте
    - [x] r4a-cli: download CA cert + install to system trust store (macOS + Linux Debian/Fedora)
    - [x] r4a-cli: remove CA cert on connect down / Ctrl-C
    - [x] compose.yaml: проброс порта 443

- [x] **Манифесты через State (Sled DB)**
    - [x] Убрать git-based manifest polling
    - [x] Добавить CRUD методы в r4a-store (put_manifest, list_manifests, delete_manifest)
    - [x] Новые эндпоинты: GET/POST/DELETE /api/manifests
    - [x] Миграция старого blob-формата манифестов
    - [x] TUI: экран Manifests (просмотр, создание, удаление)
    - [x] Web UI: CRUD форма манифестов (без TOML редактора)
    - [x] CLI: manifest_upsert, manifest_delete в r4a-client

- [x] **Fix: Worker перезапускает контейнеры в цикле**
    - [x] Label-изоляция: агенты фильтруют контейнеры по `r4a.node=<name>`
    - [x] При 409 (контейнер без лейбла) — удалять и пересоздавать с лейблом
    - [x] Fix ExposedPorts: добавить Config.ExposedPorts при создании контейнера с портами

- [x] **Security: устранение критических и высоких проблем (2026-05-21)**
    - [x] C-1: случайная соль Vault (с миграцией старых данных)
    - [x] C-2: агент API на VPN IP; мастер — VPN-only middleware (RFC-1918 + loopback)
    - [x] C-3: whitelist деревьев для /api/store/sync
    - [x] H-1: constant_time_eq для сравнения секретов
    - [x] H-2: R4A_ALLOW_MASTER_JOIN=1 для master-role join
    - [x] H-3: next_ip u8 → u16 + проверка переполнения
    - [x] H-4: секрет сервиса через EnvironmentFile (0o600), не в cmdline
    - [x] H-5: убран запуск бинарника при обновлении
    - [x] H-6: CORS ограничен VPN/localhost origin

- [x] **Containers API: stop/start (2026-05-25)**
    - [x] Агент: POST /containers/:name/stop, POST /containers/:name/start
    - [x] Сервер: прокси POST /api/nodes/:node/containers/:container/stop и /start
    - [x] Web UI (Containers.tsx): кнопки Stop (красная) / Start (зелёная) — динамически по state контейнера; все три кнопки блокируются во время pending-операции

- [x] **Fix: медленный запуск контейнеров (2026-05-25)**
    - [x] Worker: `inspect_image` перед pull — пропускать pull если образ уже есть локально

- [x] **Fix: изоляция контейнеров между агентами (2026-05-25)**
    - [x] Worker: при `node_selector = "all"` имя контейнера = `r4a-{name}-{node_name}` (избегает конфликтов на shared Docker socket в dev)
    - [x] Worker: 409-обработчик проверяет лейбл `r4a.node` перед удалением — не трогает контейнеры, созданные не через r4a
    - [x] Web UI (Manifests.tsx): убран дефолт `"all"` для node_selector; поле обязательно (красная рамка + заблокированный Save если пусто)

- [x] **Fix: Updates tab не показывает подключённых агентов**
    - [x] Server: `update_status_handler` теперь объединяет `peers` (role=agent) с `agent_update_states` — агенты отображаются сразу после join со статусом "idle"
    - [x] Web UI (Updates.tsx): добавлен статус "idle" (серый иконка Server)

- [x] **Fix: Update не работает**
    - [x] compose.yaml: `R4A_SKIP_SIGNATURE_VERIFY=1` для agent1/agent2 (без .sig → bail)
    - [x] Agent: при `self_checksum == master_checksum` репортит "updated" (не молчит) → мастер может сбросить флаг
    - [x] Agent: репортит начальный checksum + "idle" при connect
    - [x] Server: авто-сброс `update_pending` требует статус "Updated" + matching checksum (не просто checksum)
    - [x] Server: статус "unknown"/"idle" с matching checksum → показывается как "updated" в UI
    - [x] Makefile: `pkill -9 r4a-agent` → `docker restart node-agentN` (pkill не перезапускает процесс в docker)

- [x] **Feature: Connection (клиентское VPN-подключение к кластеру)**
    - [x] r4a-core: модель `Connection` (id, pubkey, vpn_ip, label, connected_at, last_seen)
    - [x] r4a-core: новый `Resource::Connections` для RBAC
    - [x] r4a-store: дерево `connections` в Sled, CRUD методы
    - [x] r4a-vpn: `add_peer` / `remove_peer` для динамического управления WG пирами
    - [x] r4a-server: POST /api/connections (создать подключение)
    - [x] r4a-server: DELETE /api/connections/:id (отключиться)
    - [x] r4a-server: GET /api/connections (список активных)
    - [x] r4a-server: POST /api/connections/:id/heartbeat (продлить жизнь)
    - [x] r4a-server: фоновая задача — удалять connections где last_seen > 90s
    - [x] r4a-cli: команды connect up/down/status/list (вместо отдельного бинарника)
    - [x] r4a-client: методы connection_create/delete/heartbeat/connections_list
    - [x] Web UI: вкладка "Connections" (таблица: IP, label, last_seen, кнопка Disconnect)

- [x] **Feature: r4a.local DNS scheme (2026-05-25)**
    - [x] r4a-cli: `connect up` добавляет `master.r4a.local → 10.42.0.1` (вместо `master.local`)
    - [x] r4a-cli: `connect up --label X` добавляет `X.r4a.local → <vpn_ip>` (клиентский IP)
    - [x] r4a-cli: `connect up` добавляет `<node_name>.r4a.local` для каждой ноды кластера
    - [x] r4a-cli: `ConnectionState` хранит `added_hosts: Vec<String>` — чистит все при disconnect
    - [x] r4a-server: CORS добавлен `http://master.r4a.local` в AllowOrigin

- [x] **Feature: встроенный DNS-сервер (2026-05-25)**
    - [x] r4a-server: `run_dns_server(vpn_ip, store)` — UDP на `<vpn_ip>:53`
    - [x] r4a-server: `*.r4a.local` — peers по name, connection labels по label, master → 10.42.0.1
    - [x] r4a-server: AAAA → NOERROR/empty, A/ANY → ответ или NXDOMAIN, остальное → forward 8.8.8.8
    - [x] r4a-vpn: `set_resolver_domain` / `remove_resolver_domain` — `/etc/resolver/<domain>` на macOS
    - [x] r4a-cli: `connect up` создаёт `/etc/resolver/r4a.local` → `10.42.0.1`; убраны node /etc/hosts
    - [x] r4a-cli: `connect down` / Ctrl-C удаляют `/etc/resolver/r4a.local`

- [x] **Feature: r4a-cli connect service install/uninstall (2026-05-26)**
    - [x] `r4a-cli connect service install [--label X] [--wg-endpoint X] [--scope user|system]`
    - [x] Linux: systemd user-service (~/.config/systemd/user/) или system (/etc/systemd/system/), EnvironmentFile=~/.r4a-connect.env (0600)
    - [x] macOS: launchd ~/Library/LaunchAgents/com.r4a.connect.plist, token в EnvironmentVariables (не в args), chmod 0600
    - [x] `r4a-cli connect service uninstall [--scope user|system]`

- [x] **Feature: r4a-telemetry MVP — централизованные логи контейнеров (2026-07-13)**
    - [x] Крейт `crates/r4a-telemetry`: LogEntry/LogBatch, LogStore (отдельный sled `~/.r4a-server/logs-db`, append-only ключи, tail-query, prune), collector (bollard follow-стрим r4a-контейнеров, батчи 2s/200 строк)
    - [x] r4a-core: `Resource::Logs` для RBAC
    - [x] r4a-server: POST /api/logs (ingest, cluster secret), GET /api/logs?node=&container=&tail= (RBAC Get Logs), GET /api/logs/containers (List), GET /api/logs/stream (SSE, token в header или ?token=), retention 3 дня/час
    - [x] r4a-agent: spawn collector после connect
    - [x] Тест на docker-кластере: логи nginx (stdout+stderr) в store, SSE live, RBAC 401/403, переподхват после рестарта контейнера
    - [x] Follow-up: вкладка Logs в Web UI (EventSource + ?token=) — сделано 2026-07-13
    - [ ] Follow-up: экран логов в TUI
    - [ ] Follow-up: LLM-трейсы / OpenTelemetry-спаны (MAIN.md §2.7)
    - [ ] Follow-up: метрики нод в telemetry-store (история CPU/RAM, сейчас только last-value в peers)

- [x] **Fix A: IP forwarding на мастере (агент↔агент трафик через хаб)**

    Контекст: агент шлёт весь 10.42.0.0/24 через мастера (`AllowedIPs = 10.42.0.0/24`),
    но мастер не форвардит пакеты между wg-пирами — `net.ipv4.ip_forward` нигде не включается.
    Сейчас agent1 не может достучаться до agent2 по VPN IP.

    - [x] `crates/r4a-vpn/src/wireguard.rs::setup_master_with_peers`: добавить в `[Interface]`
          строку `PostUp = sysctl -w net.ipv4.ip_forward=1` (только Linux; на macOS мастер
          не поддерживается в проде, пропустить)
    - [x] Проверка: `make dev-up` → `docker exec node-agent1 ping 10.42.0.3` (agent1 → agent2 через мастера)

- [x] **Fix B: корректное определение публичного endpoint мастера**

    Контекст: `get_external_ip()` (binaries/r4a-server/src/main.rs:1961) сканирует `ip -4 addr`:
    (1) на облаках с 1:1 NAT (AWS/GCP/Oracle) вернёт приватный IP — агенты получат нерабочий
    endpoint; (2) приоритет адресов `100.x` отдаст Tailscale-IP, если он есть на машине.

    - [x] CLI-аргумент `--public-endpoint <host:port>` + env `R4A_PUBLIC_ENDPOINT` для r4a-server —
          явное значение всегда выигрывает у автодетекта; валидация через `validate_endpoint`
    - [x] `get_external_ip()`: убрать приоритет `100.x`; если на интерфейсах только приватные
          RFC-1918 адреса — спросить внешний сервис (`https://api.ipify.org`, таймаут 3s),
          иначе fallback на текущее поведение; результат кэшировать (OnceLock)
    - [x] Использовать во всех трёх местах: `join_handler` (master_endpoint в ответе),
          `join_master`, регистрация себя в peers
    - [x] Проверка: `R4A_PUBLIC_ENDPOINT=1.2.3.4:51820` → в ответе `/api/join` поле
          `master_endpoint == 1.2.3.4:51820`; без env — реальный внешний IP

- [x] **Feature: синхронизация peer'ов (фундамент для P2P)**

    Контекст: агент получает список peer'ов ровно один раз при `connect` (JoinResponse.peers),
    никакого `/api/peers` нет; мастер не отслеживает наблюдаемые WG endpoint'ы агентов.
    Без этого P2P невозможен.

    - [x] r4a-vpn: параметризовать имя интерфейса в `add_peer`/`remove_peer` — сейчас
          захардкожен `wg0`, на macOS интерфейс `utunN` (брать из MacosWgState)
    - [x] r4a-vpn: `observed_endpoints()` — парсинг `wg show <iface> endpoints`
          (pubkey → ip:port после NAT; это бесплатный STUN, мастер видит реальный адрес агента)
    - [x] r4a-server: фоновая задача (каждые ~15s) — обновлять `observed_endpoint` в peers map
    - [x] r4a-server: `GET /api/peers` (VPN-only, RBAC) — PeerInfo + `public_endpoint`
          + `observed_endpoint` для каждой ноды
    - [x] r4a-core: поле `observed_endpoint: Option<String>` в PeerInfo
    - [x] r4a-agent: опция `--public-endpoint` (если у агента белый IP/проброшен порт) —
          заполнять `JoinRequest.public_endpoint` (поле уже есть, сейчас всегда None)
    - [x] r4a-agent: цикл (каждые ~30s) — GET /api/peers, поддерживать локальный кэш peer'ов
          (пока без изменения WG-конфигурации — это следующая задача)
    - [x] Проверка: на docker-кластере `curl master:8080/api/peers` показывает обоих агентов
          с observed_endpoint

- [x] **Feature: P2P-туннели агент↔агент в обход хаба (WireGuard hole punching)**

    Контекст: cryptokey routing делает маршрутизацию сам — если добавить агенту peer'а
    с `AllowedIPs <ip>/32`, этот маршрут специфичнее хабового /24 и трафик пойдёт напрямую;
    при удалении peer'а трафик автоматически возвращается через хаб. Механика hole punching:
    мастер раздаёт обеим сторонам наблюдаемые endpoint'ы друг друга, оба добавляют peer'а
    с keepalive и одновременные исходящие пакеты пробивают NAT (full-cone/restricted-cone).
    Symmetric NAT не пробивается — остаётся релей через хаб (текущее поведение).

    Зависимость: «синхронизация peer'ов» должна быть сделана.

    - [x] r4a-agent: для каждого peer'а-агента с известным endpoint (приоритет:
          public_endpoint → observed_endpoint) — `wg set <iface> peer <pub> endpoint <ep>
          allowed-ips <vpn_ip>/32 persistent-keepalive 25`
    - [x] Координация одновременности: обе стороны получают endpoint'ы из одного /api/peers
          и добавляют peer'ов в своём 30s-цикле — этого достаточно, отдельный сигнальный
          механизм не нужен (keepalive 25s держит попытки)
    - [x] Health check / fallback (критично!): каждые ~30s парсить
          `wg show <iface> latest-handshakes`; если для p2p-peer'а handshake отсутствует
          или старше 180s — `remove_peer` (маршрут откатывается на /24 через хаб),
          ретрай с экспоненциальным backoff (1m → 5m → 30m)
    - [x] Не добавлять p2p-peer'а для мастера (он и так прямой peer)
    - [x] Логи: `p2p established with <name>` / `p2p failed, falling back to hub relay`
    - [x] Проверка (docker): в одной сети direct всегда пробьётся — agent1↔agent2 ping,
          `wg show` на agent1 показывает peer agent2 со свежим handshake; убить agent2 →
          через ≤180s peer удалён, ping идёт через хаб (после Fix A)
    - [ ] Проверка (реальные NAT): asus + home за разными NAT через VPS —
          `wg show` handshake напрямую; отключить UDP между ними (firewall) → fallback на хаб
    - [ ] Follow-up: Web UI/TUI — колонка connection type (direct / relay) в списке нод
