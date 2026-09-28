---
name: rust
description: High-performance, low-abstraction Rust rules emphasizing determinism, explicit ownership, no hidden allocations in hot paths, pre-allocation, and parallel-safe patterns.
---

# Rust

## Toolchain

- Edition: 2021 or newer.
- Cargo.lock must be committed.
- Reproducible builds only.
- Lints:
  - `#![deny(warnings)]` in CI.
  - `#![deny(clippy::all, clippy::pedantic)]` unless locally justified.
  - `#![deny(unsafe_op_in_unsafe_fn)]`.

## Determinism Contract

- Identical inputs must produce identical outputs and identical observable behavior.
- Results must not depend on thread scheduling.
- Parallel reductions must use stable partitioning.
- Hash-based containers must not influence observable results unless explicitly required.
- No reliance on randomized hashing in algorithmic logic.

## Execution Phases

Code must be structured into explicit phases:

### Initialization Phase
- Allocate and size all buffers.
- Build indices and precompute lookup tables.
- Configure execution parameters.
- Validate invariants.

### Execution Phase (Hot Path)
- Compute only.
- No allocations.
- No resizing.
- No logging.
- No implicit synchronization.
- No panic paths.

### Teardown Phase
- Final aggregation.
- Resource release.
- Output formatting.

Parallelism is allowed only during execution.

## Memory and Allocation

- No dynamic allocation in hot paths.
- All `Vec` capacity must be reserved during initialization.
- No `collect`, `format!`, `String` building, or implicit allocation in kernels.
- Prefer:
  - `&[T]`
  - `&mut [T]`
  - pre-sized `Vec<T>`
- If partial initialization is required:
  - use `MaybeUninit<T>` with strict invariants.

## Error Handling

- Initialization returns `Result<T, E>`.
- Execution must be panic-free.
- No `.unwrap()` or `.expect()` in hot paths.
- Use `debug_assert!` for invariants.
- Do not use panic for control flow.

## Concurrency

- No global mutable state.
- No atomics unless explicitly requested.
- Parallel writes must target disjoint memory regions.
- No order-dependent reductions.
- Thread lifetime must be explicit (`std::thread::scope` preferred).

## Data Layout

- Prefer structure-of-arrays for throughput kernels.
- Avoid layout changes unless measured.
- `#[repr(C)]` only when required for FFI or binary layout contracts.
- Explicit alignment only when justified by profiling.

## Dispatch and Abstraction

- Prefer explicit functions and simple `struct` data carriers.
- Avoid dynamic dispatch in hot paths.
- Prefer enums + `match` over trait objects.
- Generics allowed if monomorphized and predictable.
- Avoid excessive trait layering or type-level programming.


## Avoid Trivial Wrapper Functions

- Do not introduce single-line wrapper functions that merely forward arguments to another function or method without adding meaningful semantics, invariants, ownership changes, type adaptation, synchronization, instrumentation, or abstraction.
- Prefer calling the underlying operation directly.
- Keep wrappers only when they establish a real API boundary, encode invariants, adapt ownership/lifetimes, or are required by a trait/interface contract.
- Do not create wrappers solely to make call sites shorter or visually cleaner.

## Validate Early; Keep Hot Paths Free of Redundant Checks

- Perform input validation, configuration validation, capability checks, dimensional validation, range validation, format validation, and invariant establishment during initialization or another cold outer layer.
- Prefer APIs where reaching a hot-path function implies that its preconditions have already been established.
- Once validation has completed, execution code must trust those established invariants and must not revalidate them.
- Do not repeatedly perform `Option`, `Result`, range, format, state, or capability checks inside tight loops when those conditions can be resolved before entering the hot path.
- Hoist invariant decisions out of per-element, per-pixel, per-block, and per-iteration processing.
- Prefer selecting the processing implementation once before execution rather than matching on configuration inside every iteration.
- Treat violations of established hot-path preconditions as programmer/caller errors, not recoverable per-element conditions.
- `debug_assert!` may document invariants but must impose no runtime cost in optimized builds.
- Conditional control flow intrinsic to the algorithm is allowed. The restriction applies to redundant validation, dispatch, and invariant checks.

## Hot-Path Dispatch

- Resolve algorithm, format, precision, and configuration variants before entering tight processing loops.
- Prefer dispatching once to a specialized monomorphized function over repeatedly matching on an enum inside the loop.
- Generics and const generics are encouraged when they eliminate runtime dispatch without introducing excessive abstraction or code-size growth.
- Dynamic dispatch through `dyn Trait` is forbidden in hot paths unless profiling demonstrates that the indirection is irrelevant.
- Do not sacrifice readability or produce excessive monomorphization solely to eliminate negligible branches.

## Bounds-Check Policy

- Prefer loop and slice structures that allow LLVM to prove bounds safety and eliminate redundant bounds checks.
- Prefer iteration over slices, `chunks_exact`, explicit zipped slices, or other zero-cost patterns when they produce simpler bounds reasoning than repeated indexing.
- When indexing is required, establish common lengths and valid ranges before entering the hot loop.
- Do not add redundant manual bounds checks when slice structure or established invariants already guarantee validity.
- Do not use `get_unchecked` merely to avoid assumed bounds-check overhead.
- Consider unchecked indexing only after profiling or generated-code inspection shows a meaningful remaining bounds-check cost and safe Rust cannot express the invariant efficiently.

## Iterator Policy

- Iterators are allowed when they compile to straightforward zero-cost loops and improve bounds-check elimination or clarity.
- Avoid long iterator/adaptor chains in performance-critical code when they obscure generated control flow or inhibit optimization.
- Prefer explicit loops when access patterns, vectorization, aliasing behavior, or generated code need to remain obvious.

## Collections

Hot paths may only use:
- slices
- arrays
- pre-sized `Vec`

Avoid:
- `HashMap`
- `HashSet`
- heap-backed graph structures

If ordered lookup is required, use `BTreeMap`.

## I/O and Logging

- No logging in execution phase.
- No formatting in execution phase.
- Diagnostics must be feature-gated.

## Unsafe Policy

- `unsafe` is not allowed by default.
- It may be used only when:
  - There is no safe alternative matching required performance.
  - Low-level hardware control is required.
- Before implementation:
  - The necessity of `unsafe` must be explicitly discussed.
  - The performance justification must be stated.
  - The invariants must be documented.

- Do not introduce `unsafe` solely because it appears faster.
- First inspect whether safe code already compiles to equivalent machine code.
- Use `unsafe` for performance only when profiling or generated-code inspection demonstrates a material limitation of the safe implementation.
- Prefer establishing one validated unsafe boundary around a low-level primitive over spreading unchecked operations throughout processing code.

Every `unsafe` block must include:
- Safety preconditions.
- Explanation of why they hold.
- Failure conditions.

No large unsafe regions.
Unsafe must be tightly scoped.

## Dependencies and Licensing

- No GPL/LGPL/AGPL dependencies.
- Default to `std` only.
- External crates must be:
  - actively maintained,
  - widely adopted,
  - license-compatible.
- No floating semver in production.

## Style

- Code must be readable and explicit.
- Prefer clear loops over dense iterator chains in kernels.
- Comments only for invariants or non-obvious constraints.
- No clever constructs for stylistic reasons.
- Maintainability is preferred over minimal syntax.