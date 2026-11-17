# Throughput benchmark: how many answers a second

`cargo xtask latency` ([latency baseline](latency-baseline.md)) asks one question at a time and
times it. This benchmark asks the other question: how many answers a second a server sustains on
this board before it starts to lose them or to slow down, and what that costs its cores. The aim
is a figure for qberrypi that can be set beside Pi-hole, AdGuard Home, Unbound and Blocky on the
same Raspberry Pi 3 B+, measured by one generator in one way.

## Method

### On the board, over the loopback

The Pi has Wi-Fi only. A load sent from the PC goes through the PC's Wi-Fi, the access point and
the Pi's SDIO Wi-Fi chip, and the small-packet rate of that path, not the DNS server, sets the
ceiling. So the main figure is measured **on the Pi itself, over `127.0.0.1`**: the generator and
the server share the board, and nothing else is between them.

- On qberrypi the generator is built in. `POST /api/qbp/bench` (behind the API's login) runs it on
  a thread of the blocking pool, never on the runtime's thread, against `127.0.0.1:53`, one run at
  a time; `cargo xtask bench --on-pi` drives it.
- On Raspberry Pi OS the same generator is the `qb-bench` binary, built static for the Pi and
  copied there, run against the server on `127.0.0.1:53`.

Both use the library `crates/qb-bench`, so the loads, the pass rule and the figures are the same
code.

### The generator's share of the board

The generator is a program on the same four cores. Each step of a report gives the processor time
in three parts, so that a reader can see the split:

- **gen %**: the generator's own thread, from `/proc/thread-self/stat` (`/proc/self/task/<tid>/stat`),
  in per cent of one core;
- **srv %**: the server's process, in per cent of one core. On qberrypi this is the process with the
  generator's thread taken out; for another server it is the process given with `--server-pid`;
- **core %**: each core's busy share from `/proc/stat` (the busiest one in the tables).

With `speed.runtime_cpu = 1`, as shipped, qberrypi's DNS runtime is pinned to core 1 and every pool
thread, the generator's included, runs on cores 0, 2 and 3: the generator never takes the server's
core. A server on Raspberry Pi OS is not pinned; there, pin the generator to one core
(`taskset -c 3`) so that it competes with the server in a known way, and say so in the table.

### The classes

- **cached**: eight real names (`example.com`, `iana.org`, ...) asked once each before the load,
  so that every question of it is a cache hit.
- **blocked**: a name the server blocks, the first of `doubleclick.net`, `googleadservices.com` and
  `pagead2.googlesyndication.com` that answers as blocked (`0.0.0.0`, or NXDOMAIN with no record), or
  the names given with `--blocked` (a `*` label is a fresh random label each time, for a server that
  blocks subdomains) or `--blocked-list`.
- **forwarded**: a fresh random label under six real zones of six operators, as the latency tool
  asks them. **Every forwarded question really goes to the server's upstream**, which is Quad9 for
  qberrypi and Pi-hole alike, and Quad9 throttles a source that sends it too many fresh names: on
  2026-11-12 the ramp to 200 a second left the household's address without answers for 60 to 90 s,
  and the same ramp to 100 and then 80 a second from stock Pi-hole (plain UDP) for about 30 s, for
  every device of the house. The forwarded ramp therefore starts at 25 and stops at 50 questions a
  second (the API refuses more than 100), and each run says so. qberrypi itself sends at most 50
  fresh names a second to its upstreams (1,200 at once after a quiet while), and a device at
  most 25 (300 at once), the board's own `127.0.0.1`, the
  gateway and `dns.shared_clients` excepted. A question that finds a bucket empty waits for its
  token, 600 ms at most, and only then gets SERVFAIL; a question that is already out costs no
  token (`dns::admit`; `forwards_paced` and `forwards_shed` in `/api/v1/stats`). Its figure is the upstream's as much as the
  server's, and it is not a throughput test of the board; run it rarely, and never above the
  defaults on a household's line.

### Ramp and steady load

A **step** sends at a fixed rate for one second, open loop (the rate does not slow for a slow
server, as a household's devices do not), with up to 10,000 questions in flight, each matched to
its answer by ID and lost after 2 s. A step **passes** when

- under 0.1 % of its questions were lost (a SERVFAIL or REFUSED answer counts as lost),
- its p99 is under 10 ms for cached and blocked, 1 s for forwarded (50 ms from the PC), and
- the generator sent at least 95 % of the rate: a step it could not keep up with says nothing.

Latency percentiles are nearest rank, and a lost question counts as the full 2 s wait, as in the
latency tool.

The **ramp** doubles the rate from 1,000 a second until a step fails (or 200,000 is reached), then
halves the gap between the best pass and the lowest failure until they are within 5 %, while the
run's seconds last (20 by default, at most 30 on the Pi). The **most sustained** is the answered
rate of the best step that passed. The **steady load** then runs at 80 % of it for the same
seconds, as one step, for latency at a load the server carries.

### What a run does to a live Pi

The board serves the household during a run. The run's questions come from `127.0.0.1`, which the
device table does not count, but they are counted in `queries_today` and the totals, they fill the
query log (50,000 entries: a cached ramp replaces all of it), they are offered to the front panel's
lane and the watchdog sees their answers, so a run can raise a `burst` alert, and a forwarded run a
`dga` alert, for `127.0.0.1`. A forwarded run at 50 a second for 20 s stores about 1,000 negative
answers in the 10,000-entry cache, which pushes out older ones. Run it when the household can spare
the Pi for a minute, and acknowledge such alerts afterwards.

### The figure from the PC

`cargo xtask bench --from-here --server <pi>` runs the same loads from the PC over the network. It
is the figure that the Wi-Fi allows a client, not the Pi's own, and is kept apart from it: its file
says `"source": "from-here"`, and `--compare` notes when two files come from different places.

## Running it on qberrypi

The Pi must run a build with the route (`cargo xtask update` with this build first). Then, from
the repository:

```
<command that prints the API password> | cargo xtask bench --on-pi
cargo xtask bench --from-here --server <pi's IPv4>
cargo xtask bench --compare private/bench-on-pi-<A>.json private/bench-on-pi-<B>.json
```

`--class cached` (again for more) runs fewer classes, `--seconds 30` a longer ramp and steady load,
`--forwarded-max-qps 25` a gentler forwarded ramp (100 at most). The figures go to
`private/bench-<on-pi|from-here>-<UTC time>.json` (git-ignored, mode 0600) unless `--json` names a
file. The command prints each step and then the table of the classes.

`/api/v1/stats` now carries what the board does between runs as well: `cpu.cores` (each core's busy
share over the last 10 s), `cpu.process` (qberrypi's, in per cent of one core) and `memory` (resident
set, block set and cache, in KiB). Read it before and after a run.

## Running the same generator on Raspberry Pi OS

On the PC:

```
cargo build -p qb-bench --release --target aarch64-unknown-linux-musl
scp target/aarch64-unknown-linux-musl/release/qb-bench <pi>:
```

It is one static executable, with no library to install. On the Pi, with one DNS server running on
port 53 (stop the others first), and the generator pinned to core 3:

| Server | Command on the Pi |
|---|---|
| Pi-hole (FTL) | `taskset -c 3 ./qb-bench --server-pid "$(pidof pihole-FTL)" --json pihole.json` |
| AdGuard Home | `taskset -c 3 ./qb-bench --server-pid "$(pidof AdGuardHome)" --json adguard.json` |
| Unbound | `taskset -c 3 ./qb-bench --server-pid "$(pidof unbound)" --class cached --class forwarded --json unbound.json` |
| Blocky | `taskset -c 3 ./qb-bench --server-pid "$(pidof blocky)" --json blocky.json` |

- Unbound has no block lists unless it is given some. With a list as `local-zone` entries
  (`always_nxdomain` or a `redirect` to `0.0.0.0`), add `--class blocked --blocked <a name on it>`.
- A server that blocks names with something other than `0.0.0.0` or NXDOMAIN (AdGuard Home can be
  set to answer REFUSED) fails the blocked check, which says what it answered; AdGuard Home's
  default blocking mode answers `0.0.0.0` for a list's rules, and passes.
- `--blocked <name>` names a name on that server's lists when the built-in ones are not on them.
- Each server forwards to its own upstream; give each the upstream qberrypi uses (Quad9) so the
  forwarded rows are like for like, and note when one forwards over plain UDP.

Then fetch the files and compare each with qberrypi's:

```
scp '<pi>:*.json' private/
cargo xtask bench --compare private/pihole.json private/bench-on-pi-<time>.json
```

`--compare` reads both kinds of file. It compares, per class, the most sustained (more is better)
and the steady p50, p95 and p99 (less is better), and fails if B is worse on any of them. It notes
when the steady loads ran at different rates, as they do when each server's ramp found its own
most: their latencies are then each at 80 % of its own peak. For one rate for all, run qb-bench with
`--qps N --mode steady` on every server.

## Results

Fill one row per server and class from the files' tables (`qb-bench` and `cargo xtask bench` print
them last). "max" is the most sustained answers a second (`>=` when the ramp passed at its cap);
the steady columns are the steady load at 80 % of it; srv and gen are per cent of one core, core
is the busiest core.

| Server | Class | max q/s | steady q/s | p50 ms | p95 ms | p99 ms | srv % | gen % | core % | RSS MiB |
|---|---|---|---|---|---|---|---|---|---|---|
| qberrypi | cached | | | | | | | | | |
| qberrypi | blocked | | | | | | | | | |
| qberrypi | forwarded | | | | | | | | | |
| Pi-hole | cached | | | | | | | | | |
| Pi-hole | blocked | | | | | | | | | |
| Pi-hole | forwarded | | | | | | | | | |
| AdGuard Home | cached | | | | | | | | | |
| AdGuard Home | blocked | | | | | | | | | |
| AdGuard Home | forwarded | | | | | | | | | |
| Unbound | cached | | | | | | | | | |
| Unbound | forwarded | | | | | | | | | |
| Blocky | cached | | | | | | | | | |
| Blocky | blocked | | | | | | | | | |
| Blocky | forwarded | | | | | | | | | |

And the figure the Wi-Fi allows, from the PC, kept apart:

| Server | Class | max q/s from the PC | p50 ms | p99 ms |
|---|---|---|---|---|
| qberrypi | cached | | | |
| qberrypi | blocked | | | |

## The software path on a PC

Two ignored tests in `crates/qberrypi/src/dns/hostbench.rs` profile the receive → decide → send
path on a PC, without the Pi: the service set up as the board runs it (1.2 M names blocked, the
watchdog's feed with its analysis on the listener's runtime, the panel's lane emptied as the panel
does), its upstream a fake on the loopback on a runtime of its own. Run them in release, alone:

```
cargo test -p qberrypi --release --lib hostbench -- --ignored --nocapture --test-threads=1
```

The first ramps each class against the real listener (steps of 0.5 s, p99 under 5 ms) and gives
the listener thread's own share of a core; the second times each section of one answer's path in
a replica of `serve_udp`, and the parts of the resolver on their own. Measured 2026-11-12 on the
owner's PC (12 threads, under a load average of about 19 from other work, so the figures are
rough; the Pi's Cortex-A53 cores are several times slower):

| Class | answers/s | p50 ms | p99 ms | generator % | listener thread % |
|---|---|---|---|---|---|
| cached | 90,000 | 0.018 | 0.331 | 54 | 68 |
| blocked | 125,000 | 0.020 | 1.230 | 64 | 72 |
| forwarded (fake upstream) | 18,124 | 0.754 | 1.262 | 16 | 48 |

Every ramp stopped on loss while the listener's thread was below a whole core (40 to 75 %), and
the watchdog's feed dropped about 780,000 events (it holds 1,024 and is drained every 250 ms).

Where one answer's time goes, ns per answer over 128,000 answers (one clock read taken out of
each section):

| Section | cached | % | blocked | % |
|---|---|---|---|---|
| `recv_from` (syscall) | 796 | 10.6 | 777 | 11.0 |
| `note_client` | 64 | 0.9 | 57 | 0.8 |
| decide: exempt check, `now()`, lock, lane count | 126 | 1.7 | 108 | 1.5 |
| decide: `handle_query_filtered` | 2,068 | 27.6 | 1,826 | 25.7 |
| decide: lane offer, unlock | 44 | 0.6 | 64 | 0.9 |
| `try_send_to` (syscall, loopback delivery included) | 3,392 | 45.3 | 3,296 | 46.4 |
| `note_answered` | 4 | 0.1 | 0 | 0.0 |
| `observe` (watch feed offer) | 993 | 13.3 | 969 | 13.6 |
| total | 7,489 | | 7,097 | |

Inside the resolver: `handle_query` costs 1,673 ns for a cached answer and 1,580 for a blocked
one, of which the block set's lookup in 1.2 M names is 74 to 79 ns and the statistics' record 11
(cached) to 43 (blocked) ns. The rest, about 1.5 µs, is the query read with hickory-proto's
`Message::from_vec` and the answer built and written with `Message::to_vec`.

The three hot spots, for the optimisation work that follows (nothing here is optimised):

1. **The send, 3.3 to 3.4 µs, 45 % of an answer.** One `sendto` a datagram; on the loopback it also
   carries the kernel's delivery to the receiver, so on the board's Wi-Fi it is less, but it is
   still one syscall an answer. `sendmmsg`/`recvmmsg` batching is the lever, with the receive at
   0.8 µs as the other half.
2. **The resolver's parse and encode, about 1.5 µs of `handle_query`'s 1.6 to 2.1 µs, 26 to 28 %.**
   A full hickory-proto `Message` is decoded for every query and encoded for every answer; the
   lookup and the statistics are a tenth of it. Reading the question in place and patching a
   stored answer's ID and TTLs would remove most of it.
3. **The watchdog's `observe`, about 1 µs, 13 %, on the receive loop.** Every answer is parsed
   again into an event for the feed, also when the feed is full and the event is dropped, as it was
   for most of the ramp. The loss before a full core in the ramps also points at the listener's
   receive buffer, which qberrypi leaves at the kernel's default (`SO_RCVBUF` is not raised).
