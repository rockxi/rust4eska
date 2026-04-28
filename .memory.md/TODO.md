# TODO

- [x] Очистить старый проект (r4e бинарники, wg0)
- [x] Rust workspace r4a создан (r4a-server, r4a-agent, r4a-vpn)
- [x] **WireGuard VPN**: asus=master(10.42.0.1), home=agent — поднимается автоматически бинарниками
- [x] **Ingress**: r4a-server слушает 10.42.0.1:80 (встроенный axum, без nginx)
- [x] **DNS на home**: master.local → 10.42.0.1 прописывается r4a-agent в /etc/hosts
- [x] Проверка: curl http://master.local с home → HTTP 200 ✓
- [x] **TUI**: r4a-tui бинарник, Dashboard (CPU, RAM, VRAM, имя ноды), остальные экраны — заглушки
- [x] **Метрики агента**: агент шлёт CPU/RAM/VRAM на мастер каждые 5 сек через POST /api/metrics

## TUI — оставшиеся экраны

- [ ] **TUI: RBAC экран** — таблица пользователей, формы создания токенов (требует `/api/users` и `/api/tokens` на сервере)
- [ ] **TUI: Manifests экран** — список TOML-манифестов, редактор, выбор node_selector (требует `/api/manifests` на сервере)
- [ ] **TUI: Observability экран** — WebSocket-поток логов/трейсов, цветной парсинг JSON (требует r4a-telemetry + WebSocket endpoint на сервере)

## Git-хранилище манифестов

- [x] **r4a-git-registry**: крейт создан, init_repo + git http-backend CGI
- [x] **r4a-server init**: создаёт `~/.r4a-server/git/manifests.git`, маршрут `/git/*`
- [x] Задеплоено на asus

## Персистентность мастера (миграция без разрыва соединений)

- [x] **identity.json**: keypair мастера персистируется — рестарт не меняет публичный ключ
- [x] **peers.json**: список пиров восстанавливается при старте, WireGuard поднимается с ними
- [x] Повторный join по тому же pub_key возвращает тот же VPN IP
- [x] r4a-server не занимает порт 80 (только 8080)
- [x] r4a-agent и r4a-tui обновлены на порт 8080

## Каскадное обновление агентов

- [x] r4a-server: эндпоинты /api/agent-binary, /api/agent-checksum, /api/update/*
- [x] r4a-agent: auto-update loop (poll каждые 30 сек, self-replace + exit)
- [x] r4a-tui: экран Update (t=test, u=trigger)
- [x] Задеплоено на asus + home

## TUI — Git экран

- [x] r4a-server: GET /api/git/repos — листинг bare-репозиториев
- [x] r4a-tui: Screen::Git — вкладка с именами и clone URL репозиториев
- [x] r4a-server: POST /api/git/repos — создание нового bare-репозитория
- [x] r4a-tui: клавиша `n` на Git-экране — ввод имени и создание репозитория

## Следующие шаги (backend)

- [ ] r4a-store: Raft консенсус + Sled БД
- [ ] r4a-ingress: Pingora вместо Axum
