
# Aeron Channel URI Guide

Aeron channels are described by URIs specifying **where** and **how** messages flow between publishers and subscribers. Understanding these URIs is vital for configuring high-performance messaging.

## 1. General URI Format

```
aeron:<media>?key1=value1|key2=value2|...
```

Where:
- **`<media>`** is typically `udp` or `ipc`.
- **`key=value`** pairs configure various parameters.

### Common Media Types
1. **`udp`**: For unicast or multicast over UDP.
2. **`ipc`**: For Inter-Process Communication on the same machine.
3. **`spy`**: Special subscriber mode that “spies” on an existing publication in the same media driver:
   ```
   aeron:spy:aeron:<media>?key=value|...
   ```

---

## 2. Common URI Parameters

| **Parameter**       | **Description**                                                                                                  | **Example**                                |
|---------------------|------------------------------------------------------------------------------------------------------------------|--------------------------------------------|
| `endpoint`          | IP address and port for **unicast** or **multicast**. <br/>Publisher uses it to send, subscriber to receive.     | `endpoint=127.0.0.1:40123`                |
| `interface`         | The local interface (IP + port) used when sending or receiving, often for multicast.                             | `interface=192.168.1.5:40123`             |
| `control`           | (Multicast) Control address for manual control mode.                                                             | `control=224.0.1.1:40457`                 |
| `control-mode`      | How control addresses are set for multicast. <br/>- `dynamic` (default) <br/>- `manual` (requires `control`).    | `control-mode=manual`                     |
| `session-id`        | Forces a specific session ID. Used by publishers/subscribers if you need explicit session alignment.             | `session-id=12345`                        |
| `ttl`               | Time-to-live for multicast packets.                                                                              | `ttl=4`                                   |
| `linger`            | How long (ms) to linger after `close()` before freeing resources.                                                | `linger=1000`                             |
| `flow-control`      | Flow control strategy (`min`, `max`, `tagged`, or custom).                                                       | `flow-control=min`                        |
| `congestion-control`| Congestion control algorithm. Defaults to `default`, can be a class name.                                         | `congestion-control=default`              |
| `rejoin`            | Whether a subscriber auto-rejoins if publication restarts. <br/>- `true` or `false`.                            | `rejoin=false`                            |
| `tether`            | Tethering strategy for subscribers in a group. <br/>- `reliable` or `logical`.                                   | `tether=reliable`                         |
| `eos`               | End-of-stream signal. <br/>- `true` or `false`.                                                                  | `eos=true`                                |
| `term-length`       | Size of the term buffer in bytes (power of two).                                                                 | `term-length=65536`                       |
| `mtu-length`        | Maximum Transmission Unit in bytes (aligned to 32, e.g., 1408 for typical UDP).                                  | `mtu-length=1408`                         |
| `socket-rcvbuf-length` | OS-level receive buffer size override (bytes).                                                               | `socket-rcvbuf-length=2097152`            |
| `socket-sndbuf-length` | OS-level send buffer size override (bytes).                                                                  | `socket-sndbuf-length=2097152`            |
| `tags`              | Attach tags for administrative or debugging use.                                                                 | `tags=1,2,3`                              |

> **Note**  
> Not all parameters are valid or meaningful for every scenario. For example, `control` and `control-mode` only apply to **multicast**; `session-id` is often used by **publishers** or **subscribers** needing explicit session handling.

---

## 3. Examples

### 3.1 Unicast UDP Publisher
```
aeron:udp?endpoint=127.0.0.1:40123
```
- Publisher sends to `127.0.0.1:40123`.
- A subscriber could use the same URI to listen on that port.

### 3.2 Unicast UDP Subscriber
```
aeron:udp?endpoint=127.0.0.1:40123
```
- Same endpoint, used in a subscriber context.

### 3.3 Multicast Publisher (Manual Control Mode)
```
aeron:udp?endpoint=224.0.1.1:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4
```
- Joins the multicast group at `224.0.1.1:40456`.
- Uses `224.0.1.1:40457` for control messages.

### 3.4 Multicast Subscriber
```
aeron:udp?endpoint=224.0.1.1:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4
```
- Subscribes to the same group.

### 3.5 IPC Channel
```
aeron:ipc
```
- Fast intra-process communication when publisher and subscriber share the same driver.

### 3.6 Specifying an Interface
```
aeron:udp?endpoint=127.0.0.1:40123|interface=192.168.1.5:40123
```
- Binds to `192.168.1.5` for sending/receiving while targeting `127.0.0.1:40123`.

### 3.7 Reliable Multicast with Flow Control
```
aeron:udp?endpoint=224.0.1.1:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4|linger=1000|flow-control=min
```
- Uses the `min` flow control strategy and a linger time of 1 second.

### 3.8 Forced Session-ID Publisher
```
aeron:udp?endpoint=127.0.0.1:40124|session-id=555
```
- Publisher sets an explicit session ID.
- The subscriber would need `session-id=555` if it wants to receive only this session.

### 3.9 Spy on a UDP Publication
```
aeron:spy:aeron:udp?endpoint=127.0.0.1:40123
```
- Observes an existing UDP publication **within the same media driver**.

### 3.10 Spy on an IPC Publication
```
aeron:spy:aeron:ipc
```
- Observes an existing IPC publication locally.

---

## 4. Publisher vs. Subscriber vs. Spy

1. **Publisher**
    - `aeron:<media>?endpoint=<host>:<port>` (UDP) or `aeron:ipc` (IPC).
    - May specify advanced parameters (`session-id`, `flow-control`, `congestion-control`).

2. **Subscriber**
    - Similar URI, typically matches the publisher’s `endpoint` (for UDP).
    - For multicast, match `endpoint`/`control`/`control-mode`.
    - May filter by `session-id`.

3. **Spy**
    - No network socket. Taps into an existing publication in the same driver.
    - `aeron:spy:aeron:<media>?...`.

---

## 5. Conclusion

By following these guidelines, you can confidently configure Aeron channels for **publishers**, **subscribers**, and **spies**. The key is matching media types and parameters correctly:

- **Unicast** vs. **multicast**: Use `endpoint`, `control`, `control-mode`.
- **IPC**: Just use `aeron:ipc`.
- **Spy**: `aeron:spy:aeron:<media>?...` to observe a local publication.

For further details on channel configuration, or to see the latest enhancements, consult the [official Aeron Channels Wiki](https://github.com/real-logic/aeron/wiki/Channels).

---

_**Last updated:** Based on Aeron documentation available at the time of writing._