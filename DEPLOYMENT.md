# Deployment Guide

This guide covers deploying Shiioo in production environments.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Docker](#docker)
- [Kubernetes](#kubernetes)
- [Systemd (Linux)](#systemd-linux)
- [Cloud Platforms](#cloud-platforms)
- [Configuration](#configuration)
- [Monitoring](#monitoring)
- [Security](#security)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

### Required
- **Encryption Key**: 32-byte key for AES-256 encryption of secrets
  ```bash
  openssl rand -base64 32 | head -c 32
  ```

### Recommended
- Persistent storage for data directory
- TLS termination (via ingress, load balancer, or reverse proxy)
- Monitoring stack (Prometheus + Grafana)

---

## Docker

### Quick Start

```bash
# Generate encryption key
export SHIIOO_ENCRYPTION_KEY=$(openssl rand -base64 32 | head -c 32)

# Run with Docker
docker run -d \
  --name shiioo \
  -p 8080:8080 \
  -v shiioo-data:/data \
  -e SHIIOO_ENCRYPTION_KEY="$SHIIOO_ENCRYPTION_KEY" \
  ghcr.io/raskell-io/shiioo:latest
```

### Docker Compose

```bash
# Set encryption key
export SHIIOO_ENCRYPTION_KEY=$(openssl rand -base64 32 | head -c 32)

# Start Shiioo
docker-compose up -d

# With monitoring (Prometheus + Grafana)
docker-compose --profile monitoring up -d
```

### Build from Source

```bash
docker build -t shiioo:local .
docker run -d --name shiioo -p 8080:8080 \
  -e SHIIOO_ENCRYPTION_KEY="$SHIIOO_ENCRYPTION_KEY" \
  shiioo:local
```

---

## Kubernetes

### Prerequisites
- Kubernetes 1.24+
- kubectl configured
- (Optional) Prometheus Operator for monitoring

### Deploy with Kustomize

```bash
cd deploy/kubernetes

# 1. Create the encryption key secret
kubectl create secret generic shiioo-secrets \
  --namespace shiioo \
  --from-literal=encryption-key="$(openssl rand -base64 32 | head -c 32)" \
  --dry-run=client -o yaml > secret.yaml

# 2. Update ingress hostname in ingress.yaml
# Edit: host: shiioo.your-domain.com

# 3. Deploy
kubectl apply -k .

# 4. Check status
kubectl -n shiioo get pods
kubectl -n shiioo logs -f deployment/shiioo
```

### Manual Deployment

```bash
cd deploy/kubernetes

# Apply resources in order
kubectl apply -f namespace.yaml
kubectl apply -f secret.yaml      # Edit first!
kubectl apply -f configmap.yaml
kubectl apply -f pvc.yaml
kubectl apply -f deployment.yaml
kubectl apply -f service.yaml
kubectl apply -f ingress.yaml     # Edit hostname first!
```

### Scaling

```bash
# Scale replicas
kubectl -n shiioo scale deployment shiioo --replicas=3

# Enable HPA (requires metrics-server)
kubectl -n shiioo autoscale deployment shiioo \
  --min=2 --max=10 --cpu-percent=70
```

### Prometheus Operator

If using Prometheus Operator:

```bash
kubectl apply -f servicemonitor.yaml
```

---

## Systemd (Linux)

### Installation

```bash
cd deploy/systemd

# Build the binary
cargo build --release

# Run the install script as root
sudo ./install.sh
```

### Manual Installation

```bash
# 1. Create user and directories
sudo useradd --system --shell /bin/false shiioo
sudo mkdir -p /var/lib/shiioo/data /etc/shiioo

# 2. Copy binary
sudo cp target/release/shiioo /usr/local/bin/
sudo chmod 755 /usr/local/bin/shiioo

# 3. Copy configuration
sudo cp deploy/systemd/shiioo.env /etc/shiioo/
sudo cp deploy/systemd/shiioo.service /etc/systemd/system/

# 4. Set permissions
sudo chown -R shiioo:shiioo /var/lib/shiioo
sudo chmod 600 /etc/shiioo/shiioo.env

# 5. Edit configuration
sudo nano /etc/shiioo/shiioo.env
# Set SHIIOO_ENCRYPTION_KEY

# 6. Enable and start
sudo systemctl daemon-reload
sudo systemctl enable shiioo
sudo systemctl start shiioo
```

### Management Commands

```bash
# Status
sudo systemctl status shiioo

# Logs
sudo journalctl -u shiioo -f

# Restart
sudo systemctl restart shiioo

# Stop
sudo systemctl stop shiioo
```

---

## Cloud Platforms

### AWS (ECS/Fargate)

1. **Create ECR repository and push image**
   ```bash
   aws ecr create-repository --repository-name shiioo
   docker tag shiioo:latest $AWS_ACCOUNT.dkr.ecr.$REGION.amazonaws.com/shiioo:latest
   docker push $AWS_ACCOUNT.dkr.ecr.$REGION.amazonaws.com/shiioo:latest
   ```

2. **Store encryption key in Secrets Manager**
   ```bash
   aws secretsmanager create-secret \
     --name shiioo/encryption-key \
     --secret-string "$(openssl rand -base64 32 | head -c 32)"
   ```

3. **Create ECS task definition** with:
   - Container image from ECR
   - Environment variable from Secrets Manager
   - EFS volume for persistent storage
   - Health check: `/health/live`

4. **Create ECS service** with:
   - Application Load Balancer
   - Target group health check: `/health/ready`
   - Auto-scaling based on CPU/memory

### Google Cloud (Cloud Run)

```bash
# Build and push
gcloud builds submit --tag gcr.io/$PROJECT_ID/shiioo

# Create secret
echo -n "$(openssl rand -base64 32 | head -c 32)" | \
  gcloud secrets create shiioo-encryption-key --data-file=-

# Deploy
gcloud run deploy shiioo \
  --image gcr.io/$PROJECT_ID/shiioo \
  --platform managed \
  --port 8080 \
  --set-secrets=SHIIOO_ENCRYPTION_KEY=shiioo-encryption-key:latest \
  --min-instances=1 \
  --max-instances=10
```

### Azure (Container Apps)

```bash
# Create Container App environment
az containerapp env create \
  --name shiioo-env \
  --resource-group shiioo-rg \
  --location eastus

# Create secret
az containerapp secret set \
  --name shiioo \
  --resource-group shiioo-rg \
  --secrets encryption-key="$(openssl rand -base64 32 | head -c 32)"

# Deploy
az containerapp create \
  --name shiioo \
  --resource-group shiioo-rg \
  --environment shiioo-env \
  --image ghcr.io/raskell-io/shiioo:latest \
  --target-port 8080 \
  --ingress external \
  --env-vars SHIIOO_ENCRYPTION_KEY=secretref:encryption-key
```

---

## Configuration

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `SHIIOO_ENCRYPTION_KEY` | **Yes** | - | 32-byte AES-256 encryption key |
| `SHIIOO_DATA_DIR` | No | `./data` | Data directory path |
| `SHIIOO_HOST` | No | `127.0.0.1` | Bind address |
| `SHIIOO_PORT` | No | `8080` | Listen port |
| `SHIIOO_LOG_JSON` | No | `false` | JSON log format |
| `SHIIOO_RATE_LIMIT_PER_SECOND` | No | `10` | Rate limit (req/sec) |
| `SHIIOO_RATE_LIMIT_BURST` | No | `50` | Rate limit burst |
| `RUST_LOG` | No | `info` | Log level |

### Configuration File

Create `shiioo.toml`:

```toml
[storage]
blob_dir = "blobs"
event_log_dir = "events"
index_file = "index.redb"
```

---

## Monitoring

### Endpoints

| Endpoint | Purpose |
|----------|---------|
| `/health/live` | Liveness probe (Kubernetes) |
| `/health/ready` | Readiness probe (Kubernetes) |
| `/metrics` | Prometheus metrics |
| `/api/health` | General health check |

### Prometheus Configuration

```yaml
scrape_configs:
  - job_name: 'shiioo'
    static_configs:
      - targets: ['shiioo:8080']
    metrics_path: '/metrics'
```

### Key Metrics

- `http_requests_total` - Request count by method/path
- `http_request_duration_seconds` - Request latency histogram
- `active_connections` - Current connection count
- `workflow_executions_total` - Workflow execution count

---

## Security

### Checklist

- [ ] Use strong, unique encryption key
- [ ] Enable TLS (via ingress/load balancer)
- [ ] Run as non-root user
- [ ] Use read-only root filesystem
- [ ] Limit network access
- [ ] Enable rate limiting
- [ ] Regular security scans
- [ ] Audit logging enabled

### TLS Configuration

For production, always use TLS. Options:

1. **Kubernetes Ingress** with cert-manager
2. **Cloud Load Balancer** with managed certificates
3. **Reverse Proxy** (nginx, Caddy) with Let's Encrypt

---

## Troubleshooting

### Common Issues

**Container won't start**
```bash
# Check logs
docker logs shiioo
kubectl -n shiioo logs deployment/shiioo

# Verify encryption key is set
echo $SHIIOO_ENCRYPTION_KEY | wc -c  # Should be 32
```

**Health check failing**
```bash
# Test liveness
curl http://localhost:8080/health/live

# Test readiness
curl http://localhost:8080/health/ready

# Check dependencies
curl http://localhost:8080/api/health/status
```

**Permission denied errors**
```bash
# Check data directory permissions
ls -la /var/lib/shiioo/data

# Fix ownership
sudo chown -R shiioo:shiioo /var/lib/shiioo
```

**High memory usage**
```bash
# Check metrics
curl http://localhost:8080/metrics | grep memory

# Adjust limits in deployment
resources:
  limits:
    memory: 2Gi
```

### Debug Logging

```bash
# Enable debug logs
export RUST_LOG=shiioo=debug,tower_http=debug

# Or in Kubernetes
kubectl -n shiioo set env deployment/shiioo RUST_LOG=shiioo=debug
```

---

## Support

- **Issues**: https://github.com/raskell-io/shiioo/issues
- **Discussions**: https://github.com/raskell-io/shiioo/discussions
