# Makefile for r4a (rust4eska)
# Automated build, deploy and restart

TARGET = x86_64-unknown-linux-musl
BIN_SERVER = r4a-server
BIN_AGENT = r4a-agent
BIN_TUI = r4a-tui
BIN_CLI = r4a-cli
BIN_WEB = r4a-web

HOST_MASTER = asus
HOST_AGENT = home

LOCAL_BIN_SERVER_MUSL = target/$(TARGET)/release/$(BIN_SERVER)
LOCAL_BIN_AGENT_MUSL = target/$(TARGET)/release/$(BIN_AGENT)
LOCAL_BIN_TUI_MUSL = target/$(TARGET)/release/$(BIN_TUI)
LOCAL_BIN_CLI_MUSL = target/$(TARGET)/release/$(BIN_CLI)
LOCAL_BIN_WEB_MUSL = target/$(TARGET)/release/$(BIN_WEB)

# Detect architecture for local development
UNAME_M := $(shell uname -m)
ifeq ($(UNAME_M),arm64)
    DEV_TARGET = aarch64-unknown-linux-musl
else ifeq ($(UNAME_M),aarch64)
    DEV_TARGET = aarch64-unknown-linux-musl
else
    DEV_TARGET = x86_64-unknown-linux-musl
endif

LOCAL_BIN_SERVER_DEV = target/$(DEV_TARGET)/release/$(BIN_SERVER)
LOCAL_BIN_AGENT_DEV = target/$(DEV_TARGET)/release/$(BIN_AGENT)
LOCAL_BIN_TUI_DEV = target/$(DEV_TARGET)/release/$(BIN_TUI)
LOCAL_BIN_CLI_DEV = target/$(DEV_TARGET)/release/$(BIN_CLI)
LOCAL_BIN_WEB_DEV = target/$(DEV_TARGET)/release/$(BIN_WEB)

REMOTE_BIN_DIR = /usr/local/bin

.PHONY: all build-all clean dev-up dev-down dev-deploy prod-deploy-all prod-deploy-master prod-deploy-agent prod-deploy-tui

all: build-all

build-all: build-frontend
	cargo build --release --target $(TARGET)

build-dev: build-frontend
	cargo build --release --target $(DEV_TARGET) --bin r4a-server --bin r4a-agent --bin r4a-tui --bin r4a-cli --bin r4a-web

build-web: build-frontend
	cargo build --release --target $(TARGET) --bin r4a-web

build-frontend:
	cd binaries/r4a-web/frontend && npm install && npm run build

# --- Production Deploy (musl) ---

prod-deploy-master: build-all
	@echo "--- Deploying $(BIN_SERVER), $(BIN_CLI) and $(BIN_WEB) to $(HOST_MASTER) ---"
	scp $(LOCAL_BIN_SERVER_MUSL) $(HOST_MASTER):/tmp/
	scp $(LOCAL_BIN_CLI_MUSL) $(HOST_MASTER):/tmp/
	scp $(LOCAL_BIN_WEB_MUSL) $(HOST_MASTER):/tmp/
	ssh $(HOST_MASTER) "sudo systemctl stop $(BIN_SERVER) || true && \
		sudo pkill $(BIN_SERVER) || true && \
		sudo mv /tmp/$(BIN_SERVER) $(REMOTE_BIN_DIR)/$(BIN_SERVER) && \
		sudo mv /tmp/$(BIN_CLI) $(REMOTE_BIN_DIR)/$(BIN_CLI) && \
		sudo mv /tmp/$(BIN_WEB) $(REMOTE_BIN_DIR)/$(BIN_WEB) && \
		sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_SERVER) $(REMOTE_BIN_DIR)/$(BIN_CLI) $(REMOTE_BIN_DIR)/$(BIN_WEB) && \
		sudo $(REMOTE_BIN_DIR)/$(BIN_SERVER) service enable && \
		sudo systemctl restart $(BIN_SERVER)"
	@echo "--- Master binary deployed and service restarted ---"

prod-deploy-agent: build-all
	@echo "--- Deploying $(BIN_AGENT) to $(HOST_AGENT) ---"
	scp $(LOCAL_BIN_AGENT_MUSL) $(HOST_AGENT):/tmp/
	ssh $(HOST_AGENT) "sudo systemctl stop $(BIN_AGENT) || true && \
		sudo pkill $(BIN_AGENT) || true && \
		sudo mv /tmp/$(BIN_AGENT) $(REMOTE_BIN_DIR)/$(BIN_AGENT) && \
		sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_AGENT) && \
		sudo $(REMOTE_BIN_DIR)/$(BIN_AGENT) service enable --master http://100.97.158.58:3501 && \
		sudo systemctl restart $(BIN_AGENT)"
	@echo "--- Agent binary deployed and service restarted ---"

prod-deploy-tui: build-all
	@echo "--- Deploying $(BIN_TUI) and $(BIN_CLI) to all hosts ---"
	scp $(LOCAL_BIN_TUI_MUSL) $(HOST_MASTER):/tmp/
	scp $(LOCAL_BIN_CLI_MUSL) $(HOST_MASTER):/tmp/
	ssh $(HOST_MASTER) "sudo mv /tmp/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_TUI) && sudo mv /tmp/$(BIN_CLI) $(REMOTE_BIN_DIR)/$(BIN_CLI) && sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_CLI)"
	scp $(LOCAL_BIN_TUI_MUSL) $(HOST_AGENT):/tmp/
	scp $(LOCAL_BIN_CLI_MUSL) $(HOST_AGENT):/tmp/
	ssh $(HOST_AGENT) "sudo mv /tmp/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_TUI) && sudo mv /tmp/$(BIN_CLI) $(REMOTE_BIN_DIR)/$(BIN_CLI) && sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_CLI)"
	@echo "--- TUI and CLI deployed to all hosts ---"

prod-deploy-all: prod-deploy-master prod-deploy-agent prod-deploy-tui

# --- Development Cluster (Docker) ---

dev-up:
	docker compose up -d --build

dev-down:
	docker compose down

dev-deploy: build-dev
	@echo "--- Deploying to Local Docker Cluster ($(DEV_TARGET)) ---"
	docker cp $(LOCAL_BIN_SERVER_DEV) node-master:$(REMOTE_BIN_DIR)/$(BIN_SERVER)
	docker cp $(LOCAL_BIN_AGENT_DEV) node-master:$(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker cp $(LOCAL_BIN_TUI_DEV) node-master:$(REMOTE_BIN_DIR)/$(BIN_TUI)
	docker cp $(LOCAL_BIN_CLI_DEV) node-master:$(REMOTE_BIN_DIR)/$(BIN_CLI)
	docker cp $(LOCAL_BIN_WEB_DEV) node-master:$(REMOTE_BIN_DIR)/$(BIN_WEB)
	docker exec node-master chmod +x $(REMOTE_BIN_DIR)/$(BIN_SERVER) $(REMOTE_BIN_DIR)/$(BIN_AGENT) $(REMOTE_BIN_DIR)/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_CLI) $(REMOTE_BIN_DIR)/$(BIN_WEB)
	docker restart node-master

	docker cp $(LOCAL_BIN_AGENT_DEV) node-agent1:$(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker cp $(LOCAL_BIN_TUI_DEV) node-agent1:$(REMOTE_BIN_DIR)/$(BIN_TUI)
	docker cp $(LOCAL_BIN_CLI_DEV) node-agent1:$(REMOTE_BIN_DIR)/$(BIN_CLI)
	docker exec node-agent1 chmod +x $(REMOTE_BIN_DIR)/$(BIN_AGENT) $(REMOTE_BIN_DIR)/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_CLI)
	docker restart node-agent1

	docker cp $(LOCAL_BIN_AGENT_DEV) node-agent2:$(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker cp $(LOCAL_BIN_TUI_DEV) node-agent2:$(REMOTE_BIN_DIR)/$(BIN_TUI)
	docker cp $(LOCAL_BIN_CLI_DEV) node-agent2:$(REMOTE_BIN_DIR)/$(BIN_CLI)
	docker exec node-agent2 chmod +x $(REMOTE_BIN_DIR)/$(BIN_AGENT) $(REMOTE_BIN_DIR)/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_CLI)
	docker restart node-agent2
	@echo "--- Binaries updated and services restarted in Docker ---"

# --- Common ---

clean:
	cargo clean
