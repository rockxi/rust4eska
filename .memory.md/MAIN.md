Low-Level Design (LLD): rust4eska (r4a)

1. Архитектура репозитория (Cargo Workspace)

Проект разделен на рабочее пространство (workspace), состоящее из библиотечных крейтов (core-логика) и бинарных крейтов (точки входа).

r4a-workspace/
├── Cargo.toml                  # Workspace-конфигурация, общие зависимости
├── binaries/
│   ├── r4a-server/             # Бинарник Master-ноды
│   ├── r4a-agent/              # Бинарник Agent-ноды
│   └── r4a-tui/                # Бинарник консольной утилиты (r4a)
└── crates/
    ├── r4a-core/               # Базовые типы, константы, криптография
    ├── r4a-vpn/                # Сетевой уровень (WireGuard, DNS)
    ├── r4a-store/              # Raft консенсус и Sled база данных, Vault
    ├── r4a-ingress/            # Pingora-прокси, маршрутизация HTTP
    ├── r4a-git-registry/       # Gitoxide сервер и OCI Registry
    ├── r4a-worker/             # Управление Docker (Bollard), Systemd, CI/CD
    └── r4a-telemetry/          # OpenTelemetry, сборка логов и трейсов LLM


2. Детальное описание файлов и модулей

2.1. Пакет crates/r4a-core (Базовый слой)

Хранит общие структуры данных, которые переиспользуются всеми остальными крейтами. Не имеет тяжелых зависимостей.

src/lib.rs: Экспорт модулей.

src/config.rs: Парсер путей. Определяет константы ~/.r4a-server/, ~/.r4a-server/git, ~/.r4a-server/vault.

src/models/:

node.rs: Структуры Node, NodeMetrics (CPU, RAM), NodeRole (Master, Agent).

manifest.rs: Serde-определения для парсинга TOML-манифестов (контейнеры, systemd, переменные окружения, env-ссылки vault://).

rbac.rs: Структуры пользователей User, их JWT-токены и права Role, Policy.

src/crypto.rs: Обертки над крейтами aes-gcm и argon2. Функции для шифрования/расшифровки секретов Vault и генерации ключей WireGuard (X25519).

src/error.rs: Глобальные типы ошибок (с использованием крейта thiserror).

2.2. Пакет crates/r4a-vpn (Сетевой слой)

Управляет mesh-сетью и обходом NAT.

src/lib.rs: Инициализация VPN подсистемы.

src/wireguard/:

linux.rs: Интеграция с ядерным модулем WireGuard через netlink API.

macos.rs / wsl.rs: Запуск Userspace WireGuard через крейт boringtun.

peers.rs: Логика обмена публичными ключами, настройка маршрутов (CIDR 10.42.0.0/16).

src/dns/:

server.rs: Интеграция hickory-dns. Поднимает легковесный DNS-сервер на IP-адресе мастера (10.42.0.1:53) для перехвата домена master.local.

resolver.rs: (Выполняется агентом) Логика модификации /etc/resolv.conf и /etc/wsl.conf для перенаправления DNS-запросов .local в туннель.

2.3. Пакет crates/r4a-store (Состояние и Секреты)

Распределенное хранилище на базе openraft и sled.

src/lib.rs: Экспорт API хранилища.

src/raft/:

network.rs: Реализация RaftNetwork (отправка Raft-сообщений между нодами поверх HTTP/VPN).

storage.rs: Реализация RaftStorage (запись логов консенсуса в БД sled).

state_machine.rs: Бизнес-логика применения логов (Apply). Применение изменений в манифестах, RBAC, списке нод.

src/db/:

sled_wrapper.rs: Инициализация экземпляра БД sled в ~/.r4a-server/db. Разделение на Tree (таблицы): users, manifests, nodes.

src/vault.rs: Управление секретницей. Запись и чтение зашифрованных данных в ~/.r4a-server/vault. Интеграция с r4a-core/crypto.rs (in-memory расшифровка по запросу).

2.4. Пакет crates/r4a-ingress (Маршрутизация Pingora)

Сердце кластера — Ingress на базе Cloudflare pingora.

src/server.rs: Настройка pingora::server::Server. Биндинг на порты 80 и 443 интерфейса VPN (10.42.0.1).

src/proxy.rs: Динамическое построение правил маршрутизации (Reverse Proxy) на основе состояния из r4a-store. Балансировка нагрузки между агентами.

src/middleware/:

auth.rs: Проверка JWT или Basic Auth. Если запрос идет к master.local/value, master.local/git или master.local/registry, проверяет права доступа (RBAC).

src/routes/: Внутренние обработчики (Subsystems).

api.rs: REST/WebSocket API для управления кластером через TUI.

git_proxy.rs: Перенаправление трафика в r4a-git-registry.

registry_proxy.rs: Перенаправление Docker/OCI запросов.

2.5. Пакет crates/r4a-git-registry (Артефакты)

Встроенные Git и Docker Registry.

src/git/:

server.rs: Использование gitoxide. Реализация Git HTTP Smart Protocol (info/refs, git-upload-pack, git-receive-pack).

hooks.rs: Логика Post-receive хуков. При пуше генерирует событие в Raft для запуска CI/CD.

src/registry/:

oci_server.rs: Использование oci-spec-rs. Реализация API для docker push / docker pull.

storage.rs: Сохранение слоев образов (blobs) и манифестов в директорию ~/.r4a-server/registry.

cache.rs: Pull-through кэш для внешних образов (Docker Hub).

2.6. Пакет crates/r4a-worker (Исполнение нагрузок)

Работает преимущественно на Agent-нодах.

src/reconciler.rs: Цикл (Loop), который раз в N секунд опрашивает Master-ноду на предмет актуальных TOML-манифестов для текущей ноды (node_selector).

src/docker/:

runner.rs: Использование bollard. Проверка наличия образа, скачивание из master.local/registry, создание и запуск контейнера.

logs.rs: Перехват stdout/stderr контейнера через bollard и отправка в r4a-telemetry.

src/systemctl/:

manager.rs: Использование service-manager. Генерация .service или .plist файлов, вызовы systemctl daemon-reload и systemctl restart.

src/cicd/:

sandbox.rs: Изоляция (cgroups/namespaces) процесса для выполнения скриптов.

executor.rs: Парсинг bash-скриптов и их безопасный запуск с перехватом вывода. Инжект секретов (Vault) в ENV переменные процесса.

2.7. Пакет crates/r4a-telemetry (Логи и Трейсинг LLM)

Сбор и хранение метрик.

src/subscriber.rs: Кастомный tracing-subscriber. Парсит спаны (spans) в формате OpenTelemetry Semantic Conventions (LLM prompts, tokens, latency).

src/collector.rs: (На агенте) Собирает метрики с контейнеров/процессов и отправляет их на мастер через gRPC или WebSocket.

src/db/:

logs_db.rs: Отдельный экземпляр БД sled, оптимизированный для последовательной записи (Append-only). Хранит структурированные логи и трейсы.

src/streamer.rs: (На мастере) Механизм SSE / WebSockets для раздачи логов клиентам (TUI) в реальном времени.

2.8. Бинарные точки входа (Binaries)

binaries/r4a-server/src/main.rs

Точка входа для Master-ноды.

Парсинг CLI аргументов (init, init --add, service, remove) с помощью clap.

Вызов функции из r4a-vpn::wireguard для поднятия туннеля.

Инициализация баз r4a-store (Sled) и старт консенсуса openraft.

Запуск сервера r4a-ingress (Pingora) в отдельном Tokio-task.

Инициализация внутренних сервисов (git, registry, telemetry).

Если вызвана команда service: вызывает r4a-worker::systemctl для регистрации самого себя как системного демона.

binaries/r4a-agent/src/main.rs

Точка входа для Agent-ноды.

Парсинг CLI (connect, disconnect).

Обмен ключами с мастером (через REST HTTP).

Поднятие WireGuard (r4a-vpn) и настройка r4a-vpn::dns::resolver для резолвинга master.local.

Запуск r4a-worker::reconciler — начало поллинга задач (контейнеры, systemd, CI/CD).

Запуск r4a-telemetry::collector для отправки логов на мастер.

binaries/r4a-tui/src/main.rs

Утилита управления (r4a).

src/main.rs: Настройка терминала (Raw mode, alternate screen). Инициализация крейта ratatui (или cursive).

src/api_client.rs: HTTP/REST и WebSocket клиент для общения с http://master.local/api. Важно: TUI не ходит в БД Sled напрямую, он общается с API мастера.

src/ui/: Модули отображения.

dashboard.rs: Отрисовка CPU/RAM и статусов нод.

rbac.rs: Таблицы пользователей, формы создания токенов.

manifests.rs: Редактор TOML-файлов и выбор node_selector.

observability.rs: Обработчик WebSocket-потока. Цветной парсинг JSON для LLM-трейсов (подсветка промптов, токенов).

3. Ключевые процессы (Data Flows)

3.1. Как работает Инжект Секретов (Vault)

Пользователь через TUI задает секрет: r4a -> API -> r4a-store::vault шифрует AES-256 и сохраняет в sled.

В манифесте указывается: DB_PASS = "vault://production/db-pass".

Агент (r4a-worker::reconciler) скачивает манифест.

Агент видит vault:// и делает защищенный GET-запрос: http://master.local/value/production/db-pass (передавая свой токен аутентификации).

Pingora (r4a-ingress) перехватывает запрос, проверяет RBAC-права агента.

Мастер расшифровывает секрет в оперативной памяти и отдает агенту.

Агент передает секрет в bollard как переменную окружения. На диск секрет никогда не пишется.

3.2. Как работает маршрутизация к контейнеру

Разработчик запускает контейнер my-llm на порту 8080 на агенте agent-01.

Манифест сохраняется в Raft. Мастер-нода обновляет таблицы маршрутизации Pingora.

Внешний запрос поступает на мастер по адресу my-llm.master.local.

r4a-ingress::proxy смотрит в in-memory таблицу (синхронизированную с Raft), находит, что my-llm работает на agent-01 (IP 10.42.0.2).

Pingora прозрачно проксирует трафик через WireGuard туннель на 10.42.0.2:8080.
3.3. Локальный процесс разработки

Для имитации распределенного кластера на одной машине используется Docker Compose.

1.  **Подготовка окружения**:
    `make dev-up` — поднимает 3 контейнера (`node-master`, `node-agent1`, `node-agent2`) в сети Docker. Каждая нода ограничена 1 vCPU и 1GB RAM.
2.  **Цикл разработки**:
    -   Вносятся изменения в Rust-код.
    -   `make dev-deploy` — выполняет нативную кросс-компиляцию под Linux (musl) и копирует бинарники в работающие контейнеры, перезапуская процессы внутри.
3.  **Диагностика**:
    -   `docker compose logs -f` — просмотр логов кластера.
    -   `docker exec -it node-agent1 ping 10.42.0.1` — проверка VPN-связи.
    -   `docker exec -it node-agent1 r4a-tui` — запуск TUI внутри кластера.
