# 001. IO Model

> **Status:** `recommended` — start on epoll, add io_uring behind a feature flag in `v0.2`. See [ADR 0002](../06-adrs/0002-start-on-epoll.md).
> **Foundational topics:** Unix I/O multiplexing (`epoll`/`kqueue`/IOCP), `io_uring` shared-memory ring submission, DPDK / AF_XDP kernel-bypass networking
> **Related options:** 002 (async runtime), 003 (concurrency model), 004 (request queue)
> **Related ADR:** [ADR 0002](../06-adrs/0002-start-on-epoll.md)

## 1. The decision in one sentence

> Which kernel-facing IO mechanism does Riftgate's data plane use to multiplex thousands of concurrent network connections in a single Rust process?

## 2. Context — what forces this decision

Riftgate is an LLM data plane that needs to:

- Accept thousands of concurrent client HTTP/SSE streams ([NFR-S01](../01-requirements/non-functional.md): ≥50k concurrent connections in `v0.2`).
- Maintain thousands of concurrent backend connections (one per upstream model server).
- Stream tokens between clients and backends with low latency overhead ([NFR-P05](../01-requirements/non-functional.md): TTFT overhead <5 ms).
- Run on Linux as the Tier-1 platform; on macOS as Tier-2 (dev only).

Every modern OS provides multiple ways to do this. The choice has consequences for performance, code complexity, security model, deployability, and what kinds of features (zero-copy, batched submission, kernel bypass) become possible later.

This is the **first load-bearing decision in Riftgate**, and many subsequent decisions (async runtime, concurrency model, allocator strategy) cascade from it. We take the time.

## 3. Candidates

We evaluate five candidates spanning the full spectrum from "what every Linux server uses" to "what high-frequency-trading shops use":

### 3.1. `epoll` (Linux)

**What it is.** A readiness-based, level- or edge-triggered file-descriptor multiplexer. The Linux successor to `select` and `poll`. Three syscalls: `epoll_create1`, `epoll_ctl`, `epoll_wait`. Internally a red-black tree of registered fds and a ready-list updated by callbacks. O(1) for "give me ready fds among N watched."

**Why it's interesting.**
- Universal on Linux. Everyone targets it.
- Mature, well-understood, well-tooled. `strace`, `perf`, `bpftrace` all speak fluent epoll.
- Works on every Linux kernel of relevance.
- Edge-triggered (ET) mode amortizes wake-up cost when the application drains to `EAGAIN`. Production servers like nginx use ET.
- The cheapest mental model: register, wait, react.

**Where it falls short.**
- Readiness-only. The application still does `read`/`write` syscalls — one per IO operation. The syscall tax is real at very high IOPS.
- No batching of IO operations. You can batch *waiting* (one `epoll_wait` returns N events) but each subsequent `read` is a separate syscall.
- File IO is essentially blocking. `epoll` on a regular-file fd is a no-op (always ready). For Riftgate this matters less — we are network-bound — but it is a structural limitation.
- Edge-triggered correctness is harder to write. Forget to drain to `EAGAIN` and your event handler hangs until the next event. This is the most common epoll bug class.

**Real-world systems that use it.** nginx, Redis, HAProxy, Envoy (alongside other backends), Tokio (default backend on Linux), every Python `asyncio` program on Linux. Approximately the entire Linux network-server ecosystem.

**Code sketch.**
```c
int epfd = epoll_create1(EPOLL_CLOEXEC);
struct epoll_event ev = { .events = EPOLLIN | EPOLLET, .data.fd = listen_fd };
epoll_ctl(epfd, EPOLL_CTL_ADD, listen_fd, &ev);
struct epoll_event events[64];
for (;;) {
    int n = epoll_wait(epfd, events, 64, -1);
    for (int i = 0; i < n; i++) handle(events[i]);
}
```

### 3.2. `kqueue` (BSD, macOS)

**What it is.** BSD's unified event-notification interface, predating epoll by ~3 years. Single syscall (`kevent`) handles registration *and* polling. Supports not just fd readiness but also signals, vnode events, timers, and process events.

**Why it's interesting.**
- Cleaner API surface than epoll. One syscall, batch register + wait in one call.
- Unified event types across signals, timers, fds — the abstraction is broader.
- Macros and structures are well-thought-out (`EV_SET`, `EV_ADD | EV_ENABLE`, etc.).
- A genuine alternative on FreeBSD and macOS — and macOS is where most developers' laptops live.

**Where it falls short.**
- Not on Linux. Period. For a Linux-Tier-1 project, this is the disqualifier.
- macOS support is dev convenience, not production.
- Smaller real-world battle-testing than epoll at extreme scale.

**Real-world systems that use it.** Most BSD-native services; Tokio uses it on macOS; libevent and libuv have it as a backend.

### 3.3. `io_uring` (Linux 5.1+)

**What it is.** A completion-based async IO interface introduced by Jens Axboe in 2019. Two shared-memory ring buffers (Submission Queue, Completion Queue) between userspace and kernel. Submit operations as SQEs (64 bytes each), receive completions as CQEs (16 bytes each). Three syscalls: `io_uring_setup`, `io_uring_enter`, `io_uring_register`.

The interesting feature: with `IORING_SETUP_SQPOLL`, a kernel thread polls the SQ continuously, and the application can submit work with **zero syscalls**. The interesting feature behind that: registered files and registered buffers, multishot accepts, linked operations, MSG_ZEROCOPY integration.

**Why it's interesting.**
- The first true async-everything interface on Linux. Files, sockets, timers, signals — all unified.
- Massive syscall reduction. Disk-IO benchmarks reported by Axboe in *Efficient IO with io_uring* show ~100× fewer syscalls than the equivalent libaio + epoll.
- Linked operations: chain `recv → send` so the kernel fires the second when the first completes, no userspace round-trip.
- Multishot accept: register an accept once, receive a stream of completions for new connections.
- Provided buffers: kernel selects a buffer from a pool at receive time; userspace doesn't pre-allocate per-fd.
- Active development, including networking-specific features (zero-copy send, registered eventfd, etc.).

**Where it falls short.**
- **Security history.** Google's Project Zero attributed roughly 60% of Linux kernel exploits in their 2021-2023 bug-bounty disclosures to `io_uring`. Android disabled `io_uring`. Many container runtimes block it by default with seccomp. This is real and we cannot ignore it.
- **API churn.** liburing (the recommended userspace wrapper) has had multiple semantic changes; raw `io_uring` syscalls are essentially never used directly. Tracking liburing is essentially mandatory.
- **Verifier-style debugging required.** Subtle bugs (use-after-free of buffers, ordering between linked ops, partial completions) are not caught by the compiler. Production deployments need extensive test infrastructure.
- **SQPOLL costs a CPU core.** A kernel thread polling the SQ at 100% utilization is the cost of "no syscalls on submit." Worth it on dedicated hardware; expensive in shared environments.
- **Kernel version variance.** Features land continuously. A binary that depends on multishot accept or registered eventfd needs Linux 5.19+; one depending on the original interface works on 5.1+. We need to be explicit about minimums.

**Real-world systems that use it.** RocksDB (file IO), ScyllaDB / Seastar (full async stack), QEMU, FIO, nginx 1.23+ (file IO), gRPC (experimental), Tokio (`tokio-uring`).

**Code sketch (with liburing).**
```c
struct io_uring ring;
io_uring_queue_init(256, &ring, IORING_SETUP_SQPOLL);
struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
io_uring_prep_accept_multishot(sqe, listen_fd, NULL, NULL, 0);
io_uring_submit(&ring);
struct io_uring_cqe *cqe;
io_uring_wait_cqe(&ring, &cqe);
handle_new_connection(cqe->res);
io_uring_cqe_seen(&ring, cqe);
```

### 3.4. DPDK (kernel bypass, userland NIC)

**What it is.** The Data Plane Development Kit. A Linux-Foundation project that gives userland direct, zero-syscall access to NIC hardware via Poll Mode Drivers (PMDs). The kernel's network stack is bypassed entirely. Application owns memory pools (`rte_mempool`), packet buffers (`rte_mbuf`), and per-queue rings (`rte_ring`). Uses VFIO or UIO for safe userland access to PCIe devices.

**Why it's interesting.**
- The performance ceiling. Per the DPDK programmer's guide and the kernel-bypass literature, per-packet kernel overhead is ~5-10µs versus the ~67ns budget for line-rate 10 Gbps with 64-byte frames. DPDK avoids this entirely.
- 14+ million packets per second per core demonstrated in production firewalls and load balancers.
- Zero copies, zero context switches, zero interrupts (busy-polled).
- Used by NFV, financial trading, 5G core networks.

**Where it falls short.**
- **Wrong layer of abstraction for HTTP/SSE.** DPDK gives you raw Ethernet frames. We'd need to implement (or integrate) a TCP/IP stack in userland, which is a project-sized undertaking on its own.
- **Burns a full CPU core.** PMD threads spin at 100% utilization. Capacity planning becomes "cores per gateway, not requests per gateway." Power and cost implications.
- **No `iptables`, no `tcpdump`.** Loss of the entire Linux network observability story. eBPF doesn't reach the bypass path. We'd need to build our own observability for the bypass code.
- **Hugepages required.** TLB coverage matters at line rate; deployments need 2MB or 1GB hugepages configured at boot.
- **NUMA-locality-sensitive.** Cross-NUMA packet processing is ~2× slower. Topology becomes a tuning concern.

**Real-world systems that use it.** OVS-DPDK (Open vSwitch), VPP (fd.io), commercial NFV products, HFT trading systems, Cloudflare's edge layer in some deployments.

**Suitability for an LLM gateway.** Almost zero. LLM serving is bottlenecked on backend GPU work, not on packet processing. The minimum-viable deployment of DPDK costs us more (operational complexity, observability blindness) than it gains (microseconds of overhead vs milliseconds of inference).

### 3.5. AF_XDP (kernel-assisted bypass)

**What it is.** A Linux socket type (`AF_XDP`) that pairs with an XDP (eXpress Data Path) program loaded via eBPF on the NIC driver hook. Packets matching the XDP program's policy are routed via `XDP_REDIRECT` directly into a userland UMEM (user memory) ring, bypassing most of the kernel stack but staying within the kernel's security model.

**Why it's interesting.**
- A middle ground between full DPDK bypass and the standard kernel stack.
- ~8-10 million packets per second per core (significantly better than the kernel stack's ~1 Mpps; still below DPDK's ~14+).
- Keeps the kernel security model. eBPF programs are verified before load.
- Coexists with the normal stack — non-AF_XDP traffic flows through the kernel as usual.

**Where it falls short.**
- Same "wrong abstraction" problem as DPDK for HTTP/SSE — we still need a TCP stack on top.
- Less mature ecosystem than DPDK or epoll. Tooling is younger.
- Requires NIC driver support for the XDP_REDIRECT mode.

**Real-world systems that use it.** Cilium (in-kernel load balancer), some Kubernetes networking implementations, increasingly used at hyperscaler edges.

**Suitability for an LLM gateway.** Same conclusion as DPDK. The headline numbers are about packets, but our latency budget is dominated by inference, not packet processing.

## 4. Tradeoff matrix

| Property | epoll | kqueue | io_uring | DPDK | AF_XDP | Why it matters |
|----------|-------|--------|----------|------|--------|----------------|
| Linux support | Tier-1 | none | 5.1+ | userland | 4.18+ | Riftgate Tier-1 platform is Linux. |
| macOS support | none | yes | none | no | no | Dev convenience for the maintainer. |
| Maturity / battle-testing | very high | high | medium-high | high in NFV | medium | We bet our default on this. |
| Syscall cost per IO | ~1 syscall | ~1 syscall | 0 with SQPOLL | 0 (bypass) | very low | At our scale (1k QPS), syscall cost is in the noise; at 100k QPS it dominates. |
| Async file IO | no (blocking) | no | yes | n/a | n/a | Riftgate doesn't do hot-path file IO; the WAL is async via a worker. Low priority. |
| Zero-copy network paths | sendfile, splice, MSG_ZEROCOPY | similar | SEND_ZC, registered buffers | yes (PMD owns DMA) | yes (UMEM) | Saves CPU and memory bandwidth in streaming. |
| Operational visibility | excellent (`strace`, `bpftrace`, `perf`) | good | medium-good (newer tools) | poor (no `iptables`/`tcpdump` for bypass) | medium | At 3am on a pager, this matters more than throughput. |
| Security exposure | low (mature subsystem) | low | **high** (60% of recent Linux kernel CVEs) | userland (different model) | medium (eBPF verified) | We will not put Riftgate users in front of an io_uring CVE without a deliberate decision. |
| Implementation cost in Rust | low (Tokio default) | low | medium (`tokio-uring` is real but young) | very high (build a TCP stack) | very high (build a TCP stack) | Engineering capacity is finite. |
| CPU overhead model | event-driven; idle when nothing happening | event-driven | event-driven OR busy-poll (SQPOLL) | always 100% per PMD core | configurable | Riftgate runs in many environments; "always 100% CPU" is a bad default. |
| Compatibility with future eBPF integration (Options 014) | excellent | n/a (Linux only) | excellent | poor | excellent | Our differentiation pillar. |
| Compatibility with the [`AsyncIO`](../04-design/lld-io-runtime.md) trait | natural fit | natural fit | natural fit (with completion adapter) | trait would need substantial change | trait would need substantial change | Pluggability is a Riftgate principle. |

## 5. Foundational principles

**Unix I/O multiplexing.** The C10K narrative makes the case for `epoll` on Linux directly: it is O(1) for ready-set retrieval where `select` and `poll` are O(n) in the watched-fd count. The edge-triggered vs level-triggered split is well-explored in the kernel man pages and in Kegel's C10K essay; the practical posture is that most production servers should default to LT and opt in to ET only with measured benefit, with nginx as the canonical ET production server.

**`io_uring` shared-memory ring submission.** Three claims from the `io_uring` literature shape this decision:

1. Network-workload benchmarks (≈180k req/s on `epoll` vs ≈210k on `io_uring` vs ≈240k on multishot accept) — meaningful but not 10×, and not at our QPS targets for `v0.2`.
2. Axboe's disk-IO 100× syscall-reduction figure — meaningful for storage, less so for our networking-only hot path (the WAL is async via a worker, not on the request path).
3. Project Zero's security disclosures: ~60% of recent Linux kernel exploits in their 2021-2023 bug bounty involved `io_uring`, with Android disabling it and most container runtimes blocking it by default. This is the single most important caveat for an OSS data plane.

**Kernel-bypass networking (DPDK / AF_XDP).** The "when to use / when to avoid" guidance in the DPDK programmer's guide and the XDP tutorial is unambiguous: DPDK is for Mpps and µs latency demands, not for typical web/DB workloads where application logic dominates. Our application logic dominates by orders of magnitude (inference latency in seconds, packet processing in nanoseconds). The published packet-rate matrix (kernel ≈1 Mpps → AF_XDP ≈8 Mpps → XDP ≈10 Mpps drop / ≈24 Mpps redirect → DPDK ≈14 Mpps) tells us where each lives, and that none of them are needed for an LLM gateway.

## 6. Recommendation

**Start on epoll. Add io_uring behind a feature flag in `v0.2`. Reject DPDK and AF_XDP for the foreseeable future. Keep kqueue as a Tier-2 dev-convenience backend.**

The reasoning, restated:

- epoll is universal on Linux, has the best operational story, has minimal security exposure, and is good enough for at least our `v0.2` performance targets ([NFR-P02](../01-requirements/non-functional.md): P99 overhead <10 ms at 1k QPS).
- io_uring offers real wins on syscall-cost-dominated workloads (very high QPS, very many concurrent streams). We add it as a feature flag, not as the default, so users opt into the security/maturity tradeoff explicitly. The flag would be `--features io-uring` at compile time and `io_model = "io_uring"` in config at runtime.
- kqueue stays in the codebase under `cfg(target_os = "macos")` so the Riftgate binary still works on developer laptops. Not a production target.
- DPDK and AF_XDP are out of scope. Their wins are in the wrong dimension for our workload (packets per second when our ceiling is requests-per-GPU).

### Conditions under which we'd revisit

- If the io_uring security story improves materially (e.g. Google's analysis shows the rate of new CVEs has dropped sharply), io_uring becomes a candidate for default.
- If we ever offer a "private deployment, line-rate ingress" mode for very large customers, AF_XDP enters the conversation.
- If the maintainer cost of two impls (epoll + io_uring) becomes burdensome, we revisit whether to drop one.

## 7. What we explicitly reject

- **DPDK as a Riftgate backend.** Wrong abstraction (raw frames, no TCP), kills the kernel observability story (no `bpftool`, `tcpdump`, `iptables`), burns a CPU core continuously. Reconsider only if Riftgate sprouts a private "line-rate L4-ish" deployment mode, which is not on the roadmap.
- **AF_XDP as a Riftgate backend.** Same wrong-abstraction issue, plus less mature than DPDK. Reconsider in the same conditions as above.
- **`select` / `poll` as a backend.** O(n) scaling makes them disqualified at our connection counts. They are useful only for compatibility with very old systems, which we are not.
- **A custom completion-port abstraction on Windows.** No Windows support is committed; IOCP is not on the roadmap.
- **An "auto-detect best backend" runtime probe.** Implicit choice is the wrong shape for this decision; users should know what they are running.

## 8. References

1. Linux `epoll(7)` man page — https://man7.org/linux/man-pages/man7/epoll.7.html
2. Dan Kegel, *The C10K problem* — http://www.kegel.com/c10k.html
3. Jonathan Corbet, *Ringing in a new asynchronous I/O API* (LWN.net, 2019) — https://lwn.net/Articles/776703/
4. Jens Axboe, *Efficient IO with io_uring* (kernel.dk, 2019) — https://kernel.dk/io_uring.pdf
5. Google Project Zero blog posts on `io_uring` vulnerabilities (2021-2023) — https://googleprojectzero.blogspot.com/
6. Linux Foundation, *DPDK overview* and programmer's guide — https://www.dpdk.org/overview/ and https://doc.dpdk.org/guides/prog_guide/
7. Toke Høiland-Jørgensen, *XDP and AF_XDP overview* — https://github.com/xdp-project/xdp-tutorial
8. W. Richard Stevens and Stephen A. Rago, *Advanced Programming in the UNIX Environment* (3rd ed., 2013) — chapters on multiplexing primitives.
9. Michael Kerrisk, *The Linux Programming Interface* (2010) — chapters 63-64 on alternative I/O models.
