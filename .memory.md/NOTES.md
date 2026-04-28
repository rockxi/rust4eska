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
