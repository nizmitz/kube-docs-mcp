---
title: Well-Known Labels, Annotations and Taints
---

<!-- overview -->

Kubernetes reserves all labels and annotations in the kubernetes.io and k8s.io namespaces.

<!-- body -->

## Labels, annotations and taints used on API objects

### app.kubernetes.io/component

Type: Label

Example: `app.kubernetes.io/component: "database"`

Used on: All Objects (typically used on workload resources).

The component within the application architecture.

One of the [recommended labels](/docs/concepts/overview/working-with-objects/common-labels/#labels).

### app.kubernetes.io/name

Type: Label

Example: `app.kubernetes.io/name: "mysql"`

Used on: All Objects (typically used on
[workload resources](/docs/reference/kubernetes-api/workload-resources/)).

The name of the application.

### kubernetes.io/limit-ranger

Type: Annotation

Example: `kubernetes.io/limit-ranger: "LimitRanger plugin set: cpu, memory request for container nginx; cpu, memory limits for container nginx"`

Used on: Pod

This annotation records that default values were set. {{< glossary_tooltip text="LimitRanger" term_id="limitrange" >}} does this.

```yaml
apiVersion: v1
kind: Pod
```

### service.kubernetes.io/topology-mode

Type: Annotation

Example: `service.kubernetes.io/topology-mode: Auto`

Used on: Service, Endpoints and EndpointSlice

This annotation provides a way to define how Services handle network topology.

### node.kubernetes.io/not-ready {#node-kubernetes-io-not-ready}

Type: Taint

Example: `node.kubernetes.io/not-ready: "NoExecute"`

Used on: Node

The node controller detects whether a node is ready.

## Annotations used for audit

Text.
