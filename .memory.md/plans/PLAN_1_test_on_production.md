# План тестирования на production серверах

## Окружение

- **asus** (master) — rockxi-zenbook, Ubuntu 24.04, kernel 6.17, Tailscale `100.97.158.58`
- **home** (agent) — DESKTOP-HIL871U, Ubuntu 24.04 WSL2, kernel 6.6, Tailscale `100.116.148.12`
- **локально** (macOS) — r4a-cli для подключения к кластеру

---

## Шаг 0. Деплой актуальных бинарников

```bash
make prod-deploy-all
```

Проверить что сервисы запустились:

```bash
ssh asus "sudo systemctl status r4a-server"
ssh home "sudo systemctl status r4a-agent"
```

Ожидаем: статус `active (running)` у обоих.

---

## Шаг 1. Базовая связность кластера

### 1.1. Мастер отвечает на HTTP

```bash
curl -s http://100.97.158.58:8080/
```

Ожидаем: 200 или известный JSON-ответ (не connection refused).

### 1.2. Агент зарегистрировался на мастере

```bash
ssh asus "curl -s -H 'X-R4A-Secret: <master_secret>' http://localhost:8080/api/nodes"
```

Ожидаем: `home` присутствует в списке нод с ролью `agent`.

### 1.3. VPN-туннель между нодами

```bash
ssh asus "ping -c 3 10.42.0.2"   # IP агента home
```

Ожидаем: 0% packet loss.

---

## Шаг 2. r4a-cli connect (macOS → кластер)

### 2.1. Получить Bearer токен

Предварительно: токен должен быть выдан через Web UI или TUI с правами `Resource::Connections`.

### 2.2. Подключение

```bash
r4a-cli --master http://100.97.158.58:8080 --token <bearer_token> connect up --label macbook
```

Ожидаем:
- Получен VPN IP из пула 10.42.x.x
- WireGuard туннель поднят (`wg show wg0`)
- `/etc/resolver/r4a.local` создан с `nameserver 10.42.0.1`
- `/etc/hosts` содержит `10.42.0.1 master.r4a.local`

### 2.3. DNS-резолвинг

```bash
ping -c 1 master.r4a.local
ping -c 1 macbook.r4a.local   # собственный label
```

Ожидаем: резолвинг работает, оба адреса пингуются.

### 2.4. Статус подключения

```bash
r4a-cli connect status
```

Ожидаем: показывает VPN IP, label, время подключения.

---

## Шаг 3. HTTPS и CA-сертификат

### 3.1. Установка CA

При `connect up` — CA должен устанавливаться автоматически. Проверить:

```bash
security find-certificate -c "r4a" /Library/Keychains/System.keychain
```

Ожидаем: сертификат найден.

### 3.2. HTTPS доступность

```bash
curl -s https://api.master.r4a.local/
curl -s https://web.master.r4a.local/
```

Ожидаем: нет ошибки SSL (`curl: (60) SSL certificate problem`), HTTP 200.

### 3.3. Порт 443 напрямую

```bash
curl -sv https://10.42.0.1/ 2>&1 | grep -E "SSL|subject|issuer|HTTP"
```

Ожидаем: TLS handshake успешен, cert от нашего CA.

---

## Шаг 4. Web UI

Открыть в браузере:

```
http://master.r4a.local:8081
https://web.master.r4a.local
```

Проверить вкладки:
- **Nodes** — обе ноды (`asus`, `home`) видны со статусом online, CPU/RAM метрики обновляются
- **Connections** — `macbook` виден в списке, Last Seen обновляется
- **Updates** — обе ноды показывают статус (idle/updated)

---

## Шаг 5. Manifests и Worker

### 5.1. Создать тестовый манифест через Web UI

```toml
[app]
name = "test-nginx"
node_selector = "home"

[[containers]]
image = "nginx:alpine"
ports = ["8888:80"]
```

Сохранить → ожидаем 200 OK.

### 5.2. Убедиться что агент запустил контейнер

```bash
ssh home "docker ps | grep test-nginx"
```

Ожидаем: контейнер `r4a-test-nginx` в статусе `Up`.

### 5.3. Доступность контейнера через Ingress

```bash
curl -s http://master.r4a.local:8000/   # если routing настроен
# или напрямую:
curl -s http://home.r4a.local:8888/
```

### 5.4. Containers tab в Web UI

Открыть Nodes → home → Containers. Убедиться что `r4a-test-nginx` виден, кнопки Stop/Start работают.

---

## Шаг 6. Vault (секреты)

### 6.1. Создать секрет через Web UI

Вкладка Vault → добавить секрет `test-key = "hello-from-vault"`.

### 6.2. Создать манифест с vault-ссылкой

```toml
[app]
name = "test-vault"
node_selector = "home"

[[containers]]
image = "alpine"
env = { MY_SECRET = "vault://test-key" }
```

### 6.3. Проверить инжект секрета

```bash
ssh home "docker exec r4a-test-vault env | grep MY_SECRET"
```

Ожидаем: `MY_SECRET=hello-from-vault`.

---

## Шаг 7. Обновление бинарников (Update flow)

### 7.1. Триггер обновления через Web UI

Вкладка Updates → загрузить новый бинарник агента → нажать Update.

### 7.2. Проверить статус

Ожидаем: статус home переходит `idle → updating → updated`.

```bash
ssh home "r4a-agent --version"   # должна быть новая версия
```

---

## Шаг 8. Отключение r4a-cli

```bash
r4a-cli connect down
```

Проверить:
- `wg show wg0` — интерфейс удалён (или ошибка "no such device")
- `/etc/resolver/r4a.local` — файл удалён
- `/etc/hosts` — строки `master.r4a.local` и `macbook.r4a.local` удалены
- CA-сертификат удалён из keychain: `security find-certificate -c "r4a" /Library/Keychains/System.keychain` → not found
- В Web UI → Connections: `macbook` исчез (либо evict'нулся в течение 90s)

---

## Чеклист результатов

| # | Тест | Ожидаемый результат | Статус |
|---|------|---------------------|--------|
| 1.1 | Мастер HTTP | 200 | |
| 1.2 | Агент в нодах | home в списке | |
| 1.3 | VPN пинг | 0% packet loss | |
| 2.2 | connect up | WG поднят, DNS настроен | |
| 2.3 | DNS резолвинг | master.r4a.local пингуется | |
| 3.2 | HTTPS | curl без SSL ошибок | |
| 4   | Web UI | все вкладки работают | |
| 5.2 | Worker | контейнер запущен на home | |
| 6.3 | Vault inject | секрет в ENV контейнера | |
| 7.2 | Update flow | статус updated | |
| 8   | connect down | всё очищено | |
