# Droplet runbook

First deploy:
```sh
sudo mkdir -p /opt/kube-docs-mcp && sudo cp docker-compose.yml /opt/kube-docs-mcp/
sudo cp nginx/kubedocs.conf /opt/ghost/nginx/conf/kubedocs.conf
# add kubedocs.nizmitz.com to the port-80 redirect server_name in /opt/ghost/nginx/conf/default.conf
cd /opt/ghost && docker compose run --rm certbot certonly \
  --dns-cloudflare --dns-cloudflare-credentials /etc/letsencrypt/cloudflare.ini \
  --dns-cloudflare-propagation-seconds 60 -d kubedocs.nizmitz.com
cd /opt/kube-docs-mcp && docker compose pull && docker compose up -d
cd /opt/ghost && docker compose exec nginx nginx -t && docker compose exec nginx nginx -s reload
```
Renewal: existing cron `/opt/ghost/renew_certs.sh` handles every cert under live/.

Health: `docker inspect --format '{{.State.Health.Status}}' kube-docs-mcp`, `curl -s https://kubedocs.nizmitz.com/status`.
Logs: `docker logs --tail 100 kube-docs-mcp`. RAM: `docker stats --no-stream kube-docs-mcp`.

If RAM is tight (`free -m` available < 200MB): stop uptime-kuma in /opt/ghost (`docker compose stop uptime-kuma`), remove it and `status.nizmitz.com` from compose/nginx.

GitHub Actions `deploy` needs secrets: `DROPLET_HOST`, `DROPLET_USER`, `DROPLET_SSH_KEY` (deploy-only key, `command=` restricted in authorized_keys recommended) and `cosign` installed on the droplet (`https://github.com/sigstore/cosign/releases`).
