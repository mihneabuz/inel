# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`inel` is a work-in-progress io_uring async runtime for Rust (edition 2024). This is a rewrite (`v2` branch) of an earlier attempt, which lives on `main` and at `../inel-v1`. Goals: a std/tokio-like high-level API, a thread-per-core executor, and first-class access to modern io_uring features (direct files, fixed buffers, buffer select, zero-copy TX/RX, multishot ops, linked ops). A major design driver is being testable under **miri**.

Two crates in a workspace:
- `crates/uring`: low-level io_uring bindings (`sys`) plus a "managed" layer (`managed`) that exposes ops through a (as far as possible) safe API.
- `crates/inel`: the high-level runtime (executor, `File`, `TcpStream`, `futures::AsyncBufRead` impls, ...). Currently an empty stub; all real code is in `uring`.

## Performance priorities

This is meant to be a very low-overhead implementation, and that shapes the design as much as safety does:
- **Minimize allocations.** Prefer slabs, inline state and reusing memory over boxing, especially on hot paths (submit, complete, poll). An allocation on a rare path (e.g. cancellation) is acceptable, but say why it's needed.
- **Optimize struct sizes and layout.** Sizes are chosen on purpose, so check them before adding a field:
  - `Completions` slab slots are 32 bytes, so exactly 2 fit in a cache line.
  - `Key` is `#[repr(packed)]` so that `SubmissionState` (which holds it) fits in 8 bytes.

  Adding a field to hot structs like these needs a good reason. Prefer putting the information somewhere colder (or deriving it) over growing them, and mention any size change explicitly.

## Commands

`./check.sh [test filter]` is the full gate: `cargo fmt`, clippy, tests on the real kernel, tests on the fake backend, then miri over several seeds with both stacked and tree borrows. `SEEDS="1 2"` overrides the miri seeds. Miri needs `cargo +nightly`.

Individual steps:
```sh
cargo clippy --workspace --all-targets
cargo test --workspace                         # real io_uring (needs a kernel with io_uring + recent features)
cargo test -p uring managed::tests::read       # single test/module by path filter

# fake backend natively; separate target dir so differing rustflags don't rebuild everything
CARGO_TARGET_DIR=target/fake cargo test --workspace --config 'build.rustflags=["--cfg","inel_fake"]'

# miri (fake backend is selected automatically); forward the seed, miri hides the env
INEL_FAKE_SEED=2 MIRIFLAGS="-Zmiri-env-forward=INEL_FAKE_SEED -Zmiri-seed=2" cargo +nightly miri test -p uring
# add -Zmiri-tree-borrows to MIRIFLAGS for tree borrows
```

Workspace lints enable clippy `all`, `nursery` and `cargo` as warnings; keep clippy clean.

## Architecture

### Backends (`crates/uring/src/sys`)
`sys/mod.rs` holds the safe-ish wrappers over the ring: `Sqe`, `Cqe`, `IoUring` (setup options, SQ/CQ handling, `push_sqe`, `submit_and_wait`, `for_each_cqe`) and `IoBufRing` (provided-buffer rings). It is written against a private `system` module that is one of two files, chosen by `cfg(any(miri, inel_fake))`:
- `real.rs`: thin wrappers over rustix's io_uring syscalls and `mmap`.
- `fake.rs`: an in-process emulation of the kernel. A dedicated "kernel thread" executes requests (epoll-gated for stream fds), so miri sees the kernel touching user memory concurrently and can flag UAF/aliasing/data races on buffers owned by in-flight ops. It is deliberately adversarial: random request order, every request (even NOPs) completes asynchronously, user memory accessed as late as possible, consumed SQE slots filled with garbage, cancellation may return `-EALREADY`. Randomness comes from `INEL_FAKE_SEED`. Constraints under miri: regular files need explicit offsets (can't poll), non-seekable fds need offset `-1` (can't `pread`).

Anything that changes SQE prep, ring layout or supported opcodes in `sys` usually needs a matching change in `fake.rs`, or the fake/miri runs will fail. `sys/tests.rs` tests the ring wrapper directly.

### Managed layer (`crates/uring/src/managed`)
- `Uring` (`uring.rs`) owns the `IoUring`, `Completions` and `BufGroups`. Ring is configured single-issuer + defer-taskrun, so it must stay on one thread (hence thread-per-core).
- `UringProxy` is the trait the managed types (`Submission`, `ReadSelect`, `SelectGroup`, ...) depend on to reach the ring (`with(|&mut Uring| ..)`). It is deliberately the only coupling: consumers wrap their `Uring` in whatever suits them. `inel` will likely use a `ThreadLocal<RefCell<Uring>>`, while tests use an `Rc<RefCell<Uring>>` (`TestRing` in `tests/mod.rs`). Managed types must therefore be generic over `U: UringProxy` and hold a proxy (which is `Clone`), never a `&Uring` or a concrete wrapper.
- `Submission<O, U>` (`submission.rs`) is the future/stream wrapper around an `Op` (`SingleOp` / `MultiOp` / `DetachOp`). Its `Drop` is the safety linchpin: if the op was submitted, the op is turned into a `Cancellation` (`cancellation.rs`) and handed to the ring, which keeps buffers alive until the kernel's completion arrives (and issues an `AsyncCancel`).
- `Completions` (`completion.rs`) is a slab of per-op state keyed by `Key`, which is packed into the SQE `user_data` (slot in the low 32 bits, buffer group id in bits 32..48).
- Buffers (`buffer.rs`): `IoBuf` / `IoBufMut` are `unsafe` traits that convert an owned buffer to a raw form while in flight (to avoid aliasing UB while the kernel owns it) and back after completion; `View` and `Accumulating` are adapters.
- Buffer select (`group.rs`): a split handle. `BufGroup` (provided-buffer ring + its memory) is owned by the ring's slab in `BufGroups` (ids are slab slot + 1, 0 means "no group"). Users share an `Rc<BufGroupHandle>`: `SelectGroup`s, the ops and `Cancellation`s holding one, and `SelectBuf`s (picked buffers, recycled into the ring on drop). The handle caches the fields ops need (gid, size, memory) to avoid a second pointer chase. When the last handle drops, its `Drop` pushes the gid on a `DeadList`, and `Uring::wait` unregisters and frees those groups after reaping CQEs (so a cancelled op's final CQE can still recycle its buffer first). Dropping the `Uring` leaks groups that still have handles, since an op in flight may still pick from them.
- `ops.rs` defines the concrete ops (`Read`, `Write`, `ReadSelect`, offsets `Current`/`At`). New ops implement `Op::prep` and must uphold the safety contracts documented on the traits.
- `utils/`: `Slab`/`UnsafeSlab` and `Deque` are custom allocator-aware containers (`allocator-api2`).

### Testing conventions
Managed-layer tests (`managed/tests/`) run unchanged on the real kernel, the fake natively and the fake under miri, so tests must work in all three (mind the fake's fd/offset constraints above). Tests use `TestRing::drain()` to wait for all ops to finish/cancel.
