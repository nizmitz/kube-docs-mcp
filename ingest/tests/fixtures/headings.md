---
title: Annotations
---

# Annotations

Intro text.

## cert-manager.io/issuer

- [Ingress](../usage/ingress.md)
- [Gateway](../usage/gateway.md)

The name of an Issuer to acquire the certificate from.

```yaml
metadata:
  annotations:
    cert-manager.io/issuer: my-issuer
```

### Deeper heading

More details.

## Not an annotation heading

Skip this.

## cert-manager.io/cluster-issuer {#cluster-issuer}

The name of a ClusterIssuer.
