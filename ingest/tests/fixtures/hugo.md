---
title: "Hugo Page"
weight: 10
---

<!-- overview -->

Intro paragraph with {{< glossary_tooltip text="Pods" term_id="pod" >}} inside.

{{< feature-state for_k8s_version="v1.30" state="stable" >}}

## Section One

{{< note >}}
This is a note body.
{{< /note >}}

Some text with a [link](https://example.com) and `code`.

```yaml
apiVersion: v1
kind: Pod
```

{{% code_sample file="pods/simple.yaml" %}}

### Sub A

| Name | Value |
|------|-------|
| a    | 1     |
| b    | 2     |

- item one
- item two

## Section Two {#section-two}

![image](x.png)

Final words.

<table>
<tr><th>Field</th><th>Description</th></tr>
<tr><td><code>topologyKey</code></td><td>Key of node labels &lt;x&gt;</td></tr>
</table>

