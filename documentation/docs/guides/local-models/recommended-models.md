---
sidebar_position: 1
title: Goose on Ollama - Recommendations
description: "Goose and Ollama setup and Models"
---

import Tabs from '@theme/Tabs';
import TabItem from '@theme/TabItem';
import { PanelLeft, Bot } from 'lucide-react';


introduction to using Ollama with Goose goes here, link to the other installation guides for Goose as needed

## Setup

Talk about using `GOOSE_TOOLSHIM_OLLAMA_MODEL`, `GOOSE_TOOLSHIM`, `OLLAMA_TIMEOUT`, `OLLAMA_CONTEXT_LENGTH`

## Recommended Models

This list is co-written by our community, and your level of success will vary based on your hardware and platform. We cannot guarantee these will work for every Goose user. Join our [Discord community](https://discord.gg/block-opensource) to join the discussion.

**michaelneale/qwen3:latest**
- link to ollama page
- RAM/VRAM needed
- core features
- what does it do well
- what does it NOT do well
- recommended settings:
    - OLLAMA_TIMEOUT: 300
    - GOOSE_TOOLSHIM: true

**gpt-oss**
- link to ollama page
- RAM/VRAM needed for :20b and :120b variations
- core features
- what does it do well
- what does it NOT do well
- recommended settings:

**qwen2.5-coder:32b**
- link to ollama page
- RAM/VRAM needed
- core features
- what does it do well
- what does it NOT do well
- recommended settings:
    - OLLAMA_TIMEOUT: 1800
    - GOOSE_TOOLSHIM: true
    - GOOSE_TOOLSHIM_OLLAMA_MODEL: michaelneale/qwen3:latest




## Models reloaded

`qwen2.5-coder:32b` was chosen on the basis of "least amount of investigation, most amount of apparently working. Will possibly fit in GPU RAM on a 32GB Mac, with appropriate `sysctl` tuning(?)

`qwen2.5-coder:14b` is more desirable from a performance (responsiveness) & size (GPU RAM) perspective, but anec-data-ly less accurate.

`qwen3-coder:30b` is potentially more desirable across all axis, but anec-data-ly appears more inclined to give back control to the user before complete, in activities with more steps

`gpt-oss:20b` came to my attention towards the end of writing this post (hat-tip to [samrocksc](https://discord.com/channels/1287729918100246654/1410949374296457310/1412468493588369458)) and looks promising across all axis.

`gpt-oss:120b` following on from there, the 120b seems pretty interesting:
```
# From the Ollama log
<snip>
[GIN] 2025/09/03 - 10:50:10 | 200 |  3.298722875s |   192.168.0.156 | POST     "/v1/chat/completions"
<snip>
```




#### tldr: This author recommends using [the Goose release installers](https://github.com/block/goose/releases/tag/v1.7.0) and (ultimately) used [1.7.0](https://github.com/block/goose/releases/download/v1.7.0/Goose.zip) during this install.

For the cli, first grab the [download_cli.sh](https://github.com/block/goose/releases/download/v1.7.0/download_cli.sh) and then:
```
GOOSE_VERSION=v1.7.0 ~/Downloads/download_cli.sh
```


## Models

There is plenty of pre-existing content about [how to work with models](https://www.youtube.com/watch?v=9jXO6Ln7Sbw). Not all of it will help us here. 

We use the [Experimental Ollama Tool shim](../../../../docs/experimental/ollama)
```
GOOSE_TOOLSHIM=1
```

We will not use the default Tool shim model
```
GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest
```

We will increase the Ollama timeout to 30 minutes
```
OLLAMA_TIMEOUT=1800
```

We use qwen2.5-coder:32b

Meaning __we need to perform__:
```
ollama pull qwen2.5-coder:32b
```
See [https://ollama.com/library/qwen2.5-coder](https://ollama.com/library/qwen2.5-coder)

__and__:
```
ollama run michaelneale/qwen3
```
See [https://ollama.com/michaelneale/qwen3](https://ollama.com/michaelneale/qwen3)

before attempting to launch Goose.


## Configuring Goose

We [remember](../../../../goose/docs/guides/config-file) our configuration file is at `~/.config/goose/config.yaml`

And that if we choose to, we can set all required configuration at the root of that file. The specific values to add there being:
```
OLLAMA_TIMEOUT: 1800
GOOSE_TOOLSHIM: true
GOOSE_TOOLSHIM_OLLAMA_MODEL: michaelneale/qwen3:latest
```


## Launching Goose

Instead of / as well as setting the necessary values in the config file, we remember [it is also possible to pass (override)](../../../../goose/docs/guides/environment-variables#notes) these as `ENV VARS`, for example:

### CLI

```
# all one line, wrapped...
OLLAMA_TIMEOUT=1800 GOOSE_TOOLSHIM=true GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest <path/to/goose-cli-binary/>goose session
```

### Desktop

```
# all one line, wrapped...
OLLAMA_TIMEOUT=1800 GOOSE_TOOLSHIM=true GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest open -a <path/to/>goose.app
```

This can be useful when experimenting with multiple models, but is the choice of the user

