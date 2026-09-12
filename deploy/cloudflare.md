# Cloudflare setup for kubedocs.nizmitz.com

1. DNS: `A kubedocs -> <droplet IP>`, proxied (orange cloud).
2. SSL/TLS mode: Full (strict). Origin cert = Let's Encrypt via certbot (see deploy/README.md).
3. Rate limiting (Free plan allows exactly one rule):
   - Name: `mcp-post`
   - When: `(http.host eq "kubedocs.nizmitz.com" and http.request.uri.path eq "/mcp" and http.request.method eq "POST")`
   - Rate: 60 requests / 1 minute per IP
   - Action: Block, duration 10 minutes
4. Cache rule: `http.host eq "kubedocs.nizmitz.com" and not starts_with(http.request.uri.path, "/mcp") and not starts_with(http.request.uri.path, "/status") and not starts_with(http.request.uri.path, "/healthz")` -> Eligible for cache, Edge TTL 1 hour.
5. **Do not** enable Bot Fight Mode, Super Bot Fight Mode, or any Managed Challenge on this hostname. MCP clients cannot solve challenges. If WAF managed rules are on, add a skip rule for `/mcp` on "Bot" checks only.
6. Security level: Medium. Browser Integrity Check: OFF for this host (it rejects non-browser User-Agents).
