# TODO

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
