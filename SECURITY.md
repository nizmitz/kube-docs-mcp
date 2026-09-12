# Security

Report vulnerabilities privately via GitHub Security Advisories on this repository. Do not open public issues.

Scope: the `server/` binary, the published container image, the ingest pipeline and CI workflows.
The public endpoint has no authentication by design; abuse (rate limiting) is handled at Cloudflare and nginx.
Validation input is size-capped (512 KB), time-capped (2 s), and YAML parsing runs with an alias/anchor budget.

Supply chain: images are signed with cosign (keyless, GitHub OIDC) and ship an SPDX SBOM; index releases carry
sha256 sums and a sigstore bundle. All third-party GitHub Actions are pinned to commit SHAs.
