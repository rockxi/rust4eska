# Makefile for r4a (rust4eska)
# Automated build, deploy and restart

TARGET = x86_64-unknown-linux-musl
BIN_SERVER = r4a-server
BIN_AGENT = r4a-agent
BIN_TUI = r4a-tui

HOST_MASTER = asus
HOST_AGENT = home

LOCAL_BIN_SERVER = target/$(TARGET)/release/$(BIN_SERVER)
LOCAL_BIN_AGENT = target/$(TARGET)/release/$(BIN_AGENT)
LOCAL_BIN_TUI = target/$(TARGET)/release/$(BIN_TUI)

REMOTE_BIN_DIR = /usr/local/bin

.PHONY: all build-all deploy-all deploy-master deploy-agent deploy-tui clean

all: build-all

build-all:
	cargo build --release --target $(TARGET)

# --- Master (asus) ---

deploy-master: build-all
	@echo "--- Deploying $(BIN_SERVER) to $(HOST_MASTER) ---"
	scp $(LOCAL_BIN_SERVER) $(HOST_MASTER):/tmp/
	ssh $(HOST_MASTER) "sudo systemctl stop $(BIN_SERVER) || true && \
		sudo pkill $(BIN_SERVER) || true && \
		sudo mv /tmp/$(BIN_SERVER) $(REMOTE_BIN_DIR)/$(BIN_SERVER) && \
		sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_SERVER) && \
		sudo $(REMOTE_BIN_DIR)/$(BIN_SERVER) service enable && \
		sudo systemctl restart $(BIN_SERVER)"
	@echo "--- Master binary deployed and service restarted ---"

# --- Agent (home) ---

deploy-agent: build-all
	@echo "--- Deploying $(BIN_AGENT) to $(HOST_AGENT) ---"
	scp $(LOCAL_BIN_AGENT) $(HOST_AGENT):/tmp/
	ssh $(HOST_AGENT) "sudo systemctl stop $(BIN_AGENT) || true && \
		sudo pkill $(BIN_AGENT) || true && \
		sudo mv /tmp/$(BIN_AGENT) $(REMOTE_BIN_DIR)/$(BIN_AGENT) && \
		sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_AGENT) && \
		sudo $(REMOTE_BIN_DIR)/$(BIN_AGENT) service enable --master http://100.97.158.58:8080 && \
		sudo systemctl restart $(BIN_AGENT)"
	@echo "--- Agent binary deployed and service restarted ---"

# --- TUI ---

deploy-tui: build-all
	@echo "--- Deploying $(BIN_TUI) to all hosts ---"
	scp $(LOCAL_BIN_TUI) $(HOST_MASTER):/tmp/
	ssh $(HOST_MASTER) "sudo mv /tmp/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_TUI) && sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_TUI)"
	scp $(LOCAL_BIN_TUI) $(HOST_AGENT):/tmp/
	ssh $(HOST_AGENT) "sudo mv /tmp/$(BIN_TUI) $(REMOTE_BIN_DIR)/$(BIN_TUI) && sudo chmod +x $(REMOTE_BIN_DIR)/$(BIN_TUI)"
	@echo "--- TUI deployed to all hosts ---"

# --- Common ---

deploy-all: deploy-master deploy-agent deploy-tui

clean:
	cargo clean
