---
sidebar_position: 5
title: Operating System Specifics - Mac
description: "Mac hardware"
---

import Tabs from '@theme/Tabs';
import TabItem from '@theme/TabItem';
import { PanelLeft, Bot } from 'lucide-react';


## Overview

Depending on your processor & to get the best performance, you may need to [inform your Mac that it is allowed to spin it's fans fast](https://support.apple.com/en-gb/101613).

Another consideration is the amount of shared RAM the GPU has access to. Since a while now, this has been runtime configurable, with a caveat being you need [sudo](https://www.sudo.ws/) permissions on your local hardware. Other community links to this information include [poweruser](https://poweruser.forum/post/6832) or [the referenced GitHub comment](https://github.com/ggml-org/llama.cpp/discussions/2182#discussioncomment-7698315), but regardless your information source, you want to be at least aware of the below command.

```
sudo sysctl iogpu.wired_limit_mb=<you need to work out an appropriate number>
```

__NOTE:__ On macOS Sonoma (14.x) and later, the command `sysctl iogpu.wired_limit_mb` is the correct one. However, on older macOS versions like Ventura (13.x), the correct command is `sysctl debug.iogpu.wired_limit`. The older command also takes its value in __bytes__, whereas the newer one takes it in __megabytes__, so be careful with that.

__NOTE:__ Local system stability is yours to manage now.

__NOTE:__ After changing this setting, it is necessary to restart Ollama for it to react to the change.

The above command automatically resets during system boot. If you want to persist this across reboots, it is left as an exercise for the reader. This is deliberate, because you can persistently, significantly and negatively impact the working of your Mac by mis-configuring using sysctl. 

## Observing Ollama Memory & GPU usage

With sufficient memory available all models will be 100% on GPU, as seen here:
```
ollama ps
NAME                         ID              SIZE      PROCESSOR    CONTEXT    UNTIL
michaelneale/qwen3:latest    3a38aca461cc    9.6 GB    100% GPU     32768      4 minutes from now
qwen2.5-coder:32b            b92d6a0bd47e    27 GB     100% GPU     32768      2 minutes from now
qwen2.5-coder:14b            9ec8897f747e    15 GB     100% GPU     32768      3 minutes from now
qwen2.5-coder:latest         dae161e27b0e    8.0 GB    100% GPU     32768      4 minutes from now
gpt-oss:20b                  aa4295ac10c3    18 GB     100% GPU     32768      4 minutes from now
qwen3-coder:30b              ad67f85ca250    23 GB     100% GPU     32768      4 minutes from now
gpt-oss:120b                 f7f8e2f8f4e0    74 GB     100% GPU     32768      4 minutes from now
```
With a sharp eye, the reader will notice the above list is fake - MacBooks don't have that much RAM.

In a more constrained memory environment, which can be simulated using:
```
sudo sysctl iogpu.wired_limit_mb=12400
```

You may see the model being split across the CPU + GPU:
```
ollama ps
NAME                 ID              SIZE     PROCESSOR          CONTEXT    UNTIL
qwen2.5-coder:32b    b92d6a0bd47e    27 GB    53%/47% CPU/GPU    32768      4 minutes from now
```
This is undesirable, and will significantly & negatively impact the elapsed duration before the model responds. Your aim is to manage the combination of model size(s) + available system memory, such that all models used fit into GPU accessible memory.