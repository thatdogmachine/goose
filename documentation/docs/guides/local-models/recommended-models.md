---
sidebar_position: 1
title: Goose on Ollama - Start Here
description: "Goose and Ollama setup and Models"
---

import Tabs from '@theme/Tabs';
import TabItem from '@theme/TabItem';
import { PanelLeft, Bot } from 'lucide-react';

## Objective

By the time you finish reading, you should have:
- Clear steps to get `Ollama` + `Goose` + `qwen2.5-coder:32b` calling tools locally on your choice of Operating System.
- Have a way you can evaluate other models locally
- Have a way you can evaluate using, or not, the __[Experimental Ollama Tool Shim](../../experimental/ollama)__


## Prerequisites

This page anticipates that you have:

- Familiarized yourself the relevant __Operating System Specifics__ page, if one exists for your OS, see __[here](./index.md)__
    - Initially, settings mentioned here may be optional, however it is worth being aware of them before continuing
- Followed the __Ollama on__ page for your OS. Again see __[here](./index.md)__
- Goose installed using the __[Install Goose](../../getting-started/installation)__ guide
    - This guide was written using __[1.7.0](https://github.com/block/goose/releases/tag/v1.7.0)__
    - Functionality can change over time: if the guide does not work, you may want to experiment with the Goose version



## Example configuration - this has been seen to work

# TBD: Model pinning
__as a blog post this wasn't such a big issue, as an official guide it is more so: The existing instructions use model id's that are not pinned. As a consequence third parties can change what is installed over time, increasing the risk this guide is non deterministic.__

Options:

(my assumption is that if this is done on ollama side it will result in a consistent sha dependency chain. I haven't actually validated this. An alternative might be to use hugging face repositories instead, but that seems like an over-complication.)

- no-one does it, and we call it out as a risk in the guide (this is important, because most of the point of this guide is to give the consumer something that is very likely to work)
- I fork the models on ollama and create static names that will always deliver the same model
- Someone in Block does it

__My preference is for someone in Block to do that, and failing that, the first option where we make the consumer aware__



We use the __[Experimental Ollama Tool shim](../../experimental/ollama)__
```
GOOSE_TOOLSHIM=1
```

We will not use the default Tool shim model. Instead we will use one created by one of the Block team.
```
GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest
```

We will increase the Ollama timeout to 30 minutes - the rationale being: if you already waited 10 mins (default timeout) for a request to complete, the chances are you don't mind waiting up to 30 minutes. This setting should be adjusted per operator preference.
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

We __[remember](../config-file)__ our configuration file is at `~/.config/goose/config.yaml`

And that if we choose to, we can set all these configuration values for Ollama use at the root of that file. If doing so, the specific values to set there being:
```
OLLAMA_TIMEOUT: 1800
GOOSE_TOOLSHIM: true
GOOSE_TOOLSHIM_OLLAMA_MODEL: michaelneale/qwen3:latest
```


## Launching Goose

Instead of / as well as setting the necessary values in the config file, we remember __[it is also possible to pass (override)](../environment-variables#notes)__ these as `ENV VARS`, for example:

<Tabs groupId="interface">
  <TabItem value="ui" label="Goose Desktop" default>
    ```
    # all one line, wrapped...
    OLLAMA_TIMEOUT=1800 GOOSE_TOOLSHIM=true GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest open -a <path/to/>goose.app
    ```

    Or, if you elected to set the values in `~/.config/goose/config.yaml` earlier, you can simply start the Goose Desktop App in the MacOS UI.

    The `ENV VARS` approach can be useful when experimenting with multiple models, but is up to the preference of the user.
  </TabItem>

  <TabItem value="cli" label="Goose CLI">
    ```
    # all one line, wrapped...
    OLLAMA_TIMEOUT=1800 GOOSE_TOOLSHIM=true GOOSE_TOOLSHIM_OLLAMA_MODEL=michaelneale/qwen3:latest <path/to/goose-cli-binary/>goose session
    ```
  </TabItem>
</Tabs>


## Recommended Models

This list is co-written by our community, and your level of success will vary based on your hardware and platform. We cannot guarantee these will work for every Goose user. Join our [Discord community](https://discord.gg/block-opensource) to join the discussion.

It is worth remembering there has been prior analysis: __[Community-Inspired Benchmarking: The Goose Vibe Check](https://block.github.io/goose/blog/2025/03/31/goose-benchmark/)__, with `qwen2.5-coder:32b` being the highest scoring Ollama hosted model. At the time of writing, local re-tests using goose-bench have not been completed.

In the model list below, `SIZE` = `GB`

**michaelneale/qwen3:latest**
- [link to ollama page](https://ollama.com/michaelneale/qwen3)
- RAM/VRAM needed
    ```
    ollama ps michaelneale/qwen3 | awk 'NR==1{print $1,$3; next}{print $1,$3}' | column -t
    NAME                       SIZE
    michaelneale/qwen3:latest  9.6
    ```
- what does it do well
    - fulfil `GOOSE_TOOLSHIM_OLLAMA_MODEL` 
- what does it NOT do well
    - n/a
- see also
    - See also: [Goose and Qwen3 for Local Execution](https://block.github.io/goose/blog/2025/05/12/local-goose-qwen3/)


**gpt-oss:20b**
- [link to ollama page](https://ollama.com/library/gpt-oss:20b)
- RAM/VRAM needed
    ```
    ollama ps gpt-oss:20b | awk 'NR==1{print $1,$3; next}{print $1,$3}' | column -t
    NAME         SIZE
    gpt-oss:20b  18
    ```
- what does it do well
    - anec-data-ly quick to respond compared with other similar sized models
- what does it NOT do well
    - like qwen2.5-coder:32b, has been seen to get stuck composing shell commands
- see also
    - n/a

**gpt-oss:120b**
- [link to ollama page](https://ollama.com/library/gpt-oss:120b)
- RAM/VRAM needed
    ```
    ollama ps gpt-oss:120b | awk 'NR==1{print $1,$3; next}{print $1,$3}' | column -t
    NAME          SIZE
    gpt-oss:120b  74
    ```
- what does it do well
    - anec-data-ly quick to respond compared with other similar sized models. "quick" in this context means sub two minute responses, vs 4+ minute thinking time on MacBook Pro/Max/128GB type hardware
- what does it NOT do well
    - n/a
- see also
    - n/a

**qwen2.5-coder:32b**
- link to ollama page
- RAM/VRAM needed
- use in conjunction with toolshim
    - yes
- what does it do well
    - multi step activities generally progress to completion without babysitting
- what does it NOT do well
    - anec-data-ly slower to respond
    - may need additional support composing shell commands. This may be a consequence of toolshim interaction
- see also
    - n/a
   
