# Makefile for r4a (rust4eska)
# Automated build, deploy and restart

TARGET = x86_64-unknown-linux-musl
BIN_SERVER = r4a-server
BIN_AGENT = r4a-agent
BIN_TUI = r4a-tui

HOST_MASTER = asus
HOST_AGENT = home

LOCAL_BIN_SERVER_MUSL = target/$(TARGET)/release/$(BIN_SERVER)
LOCAL_BIN_AGENT_MUSL = target/$(TARGET)/release/$(BIN_AGENT)
LOCAL_BIN_TUI_MUSL = target/$(TARGET)/release/$(BIN_TUI)

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

REMOTE_BIN_DIR = /usr/local/bin

.PHONY: all build-all clean dev-up dev-down dev-deploy prod-deploy-all prod-deploy-master prod-deploy-agent prod-deploy-tui

all: build-all

build-all:
	cargo build --release --target $(TARGET)

build-dev:
	cargo build --release --target $(DEV_TARGET)

# --- Production Deploy (musl) ---

prod-deploy-master: build-all
	@echo "--- Deploying $(BIN_SERVER) to $(HOST_MASTER) ---"
	scp $(LOCAL_BIN_SERVER_MUSL) $(HOST_MASTER):/tmp/
	ssh $(HOST_MASTER) "sudo systemctl stop $(BIN_SERVER) || true && \
		sudo pkill $(BIN_SERVER) || true && \
		sudo mv /tmp/$(BIN_SERVER) $(REMOTE_BIN_DIR)/$(BIN_SERVER) && \
		sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_SERVER) && \
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
		sudo $(REMOTE_BIN_DIR)/$(BIN_AGENT) service enable --master http://100.97.158.58:8080 && \
		sudo systemctl restart $(BIN_AGENT)"
	@echo "--- Agent binary deployed and service restarted ---"

prod-deploy-tui: build-all
	@echo "--- Deploying $(BIN_TUI) to all hosts ---"
	scp $(LOCAL_BIN_TUI_MUSL) $(HOST_MASTER):/tmp/
	ssh $(HOST_MASTER) "sudo mv /tmp/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_TUI) && sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_TUI)"
	scp $(LOCAL_BIN_TUI_MUSL) $(HOST_AGENT):/tmp/
	ssh $(HOST_AGENT) "sudo mv /tmp/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_TUI) && sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_TUI)"
	@echo "--- TUI deployed to all hosts ---"

prod-deploy-all: prod-deploy-master prod-deploy-agent prod-deploy-tui

# --- Development Cluster (Docker) ---

dev-up:
	docker compose up -d --build

dev-down:
	docker compose down

dev-deploy: build-dev
	@echo "--- Deploying to Local Docker Cluster ($(DEV_TARGET)) ---"
	docker cp $(LOCAL_BIN_SERVER_DEV) r4a-master:$(REMOTE_BIN_DIR)/$(BIN_SERVER)
	docker cp $(LOCAL_BIN_AGENT_DEV) r4a-master:$(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker exec r4a-master chmod +x $(REMOTE_BIN_DIR)/$(BIN_SERVER) $(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker exec r4a-master pkill -9 r4a-server || true
	docker cp $(LOCAL_BIN_AGENT_DEV) r4a-agent1:$(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker cp $(LOCAL_BIN_AGENT_DEV) r4a-agent2:$(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker exec r4a-agent1 chmod +x $(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker exec r4a-agent2 chmod +x $(REMOTE_BIN_DIR)/$(BIN_AGENT)
	docker exec r4a-agent1 pkill -9 r4a-agent || true
	docker exec r4a-agent2 pkill -9 r4a-agent || true
	@echo "--- Binaries updated and services restarted in Docker ---"

# --- Common ---

clean:
	cargo clean
