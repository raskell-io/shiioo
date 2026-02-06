#!/bin/bash
set -euo pipefail

# Shiioo systemd installation script
# Run as root: sudo ./install.sh

SHIIOO_USER="shiioo"
SHIIOO_GROUP="shiioo"
SHIIOO_HOME="/var/lib/shiioo"
SHIIOO_CONFIG="/etc/shiioo"
SHIIOO_BIN="/usr/local/bin/shiioo"

echo "Installing Shiioo..."

# Create user and group
if ! getent group "$SHIIOO_GROUP" > /dev/null 2>&1; then
    echo "Creating group: $SHIIOO_GROUP"
    groupadd --system "$SHIIOO_GROUP"
fi

if ! getent passwd "$SHIIOO_USER" > /dev/null 2>&1; then
    echo "Creating user: $SHIIOO_USER"
    useradd --system \
        --gid "$SHIIOO_GROUP" \
        --home-dir "$SHIIOO_HOME" \
        --shell /bin/false \
        --comment "Shiioo Service" \
        "$SHIIOO_USER"
fi

# Create directories
echo "Creating directories..."
mkdir -p "$SHIIOO_HOME/data"
mkdir -p "$SHIIOO_CONFIG"

# Set permissions
chown -R "$SHIIOO_USER:$SHIIOO_GROUP" "$SHIIOO_HOME"
chmod 750 "$SHIIOO_HOME"
chmod 750 "$SHIIOO_HOME/data"

# Copy binary (assumes it's in the current directory or specify path)
if [ -f "./shiioo" ]; then
    echo "Installing binary..."
    cp ./shiioo "$SHIIOO_BIN"
    chmod 755 "$SHIIOO_BIN"
elif [ -f "../../../target/release/shiioo" ]; then
    echo "Installing binary from target/release..."
    cp ../../../target/release/shiioo "$SHIIOO_BIN"
    chmod 755 "$SHIIOO_BIN"
else
    echo "Warning: Binary not found. Please copy shiioo to $SHIIOO_BIN manually."
fi

# Copy configuration files
echo "Installing configuration..."
if [ ! -f "$SHIIOO_CONFIG/shiioo.env" ]; then
    cp shiioo.env "$SHIIOO_CONFIG/shiioo.env"
    chmod 600 "$SHIIOO_CONFIG/shiioo.env"
    chown root:$SHIIOO_GROUP "$SHIIOO_CONFIG/shiioo.env"
    echo "IMPORTANT: Edit $SHIIOO_CONFIG/shiioo.env and set SHIIOO_ENCRYPTION_KEY"
else
    echo "Configuration already exists, skipping..."
fi

# Create default config file
if [ ! -f "$SHIIOO_CONFIG/shiioo.toml" ]; then
    cat > "$SHIIOO_CONFIG/shiioo.toml" << 'EOF'
[storage]
blob_dir = "blobs"
event_log_dir = "events"
index_file = "index.redb"
EOF
    chown root:$SHIIOO_GROUP "$SHIIOO_CONFIG/shiioo.toml"
    chmod 644 "$SHIIOO_CONFIG/shiioo.toml"
fi

# Install systemd service
echo "Installing systemd service..."
cp shiioo.service /etc/systemd/system/shiioo.service
chmod 644 /etc/systemd/system/shiioo.service

# Reload systemd
systemctl daemon-reload

echo ""
echo "Installation complete!"
echo ""
echo "Next steps:"
echo "  1. Edit /etc/shiioo/shiioo.env and set SHIIOO_ENCRYPTION_KEY"
echo "  2. Enable the service: systemctl enable shiioo"
echo "  3. Start the service: systemctl start shiioo"
echo "  4. Check status: systemctl status shiioo"
echo "  5. View logs: journalctl -u shiioo -f"
