# Streaming Sorted Candidate Enumeration Brief

## Problem Summary

We have a Rust candidate feeder that enumerates strings in a bounded edit-distance neighborhood around a seed string. The feeder is used to stream candidate strings to downstream work. The core problem is to generate candidates continuously while preserving a deterministic ranking order, without introducing catastrophic computation time as edit distance grows.

The search config includes:

- `seed`: original string, for example `"patter"`.
- `alphabet`: allowed output characters, for example lowercase, alphanumeric, or symbols.
- `min_distance` / `max_distance`: inclusive edit-distance band.
- enabled operations: insert, delete, replace, swap.
- optional keyboard-neighbor map for likelihood scoring.

Candidates are internally represented as `Vec<char>`, not byte-indexed strings. Unicode correctness matters: multibyte characters must count as one character/edit unit.

## Mutation Model

One edit from a parent string can produce:

- Delete one char, cost `2`.
- Insert one alphabet char at any char boundary, cost `2`.
- Replace one char with another alphabet char:
  - cost `1` if replacement is a keyboard neighbor,
  - cost `3` otherwise.
- Swap adjacent distinct chars, cost `1`.

Swapping identical adjacent chars must not emit the same string.

A candidate at distance `d` is reached by applying `d` edits. Its likelihood cost is the accumulated mutation cost along a path. If multiple paths reach the same candidate, the ordered enumerator should use the best known cost for that candidate at that distance and emit it once.

## Existing Ordered Contract

The compatibility enumerator must preserve this order:

1. Edit distance ascending.
2. Accumulated likelihood cost ascending.
3. Lexical candidate order ascending.

Example for seed `"a"` with alphabet `['a', 'b']`, exact distance `1`, ordered output is:

```text
""
"aa"
"ab"
"ba"
"b"
```

This order is not discovery order. It comes from building all distance-1 candidates, deduplicating, sorting by `(cost, lexical)`, then emitting.

## Initial Problem

The original `CandidateEnumerator` was lazy only between completed distance layers. It emitted candidates one by one from an already-built layer, but to advance from distance `d` to `d + 1`, it did one large opaque operation:

1. Expand every parent in the current layer.
2. Generate every one-edit neighbor.
3. Deduplicate into a map of best costs.
4. Convert the map into a vector.
5. Sort the whole vector.
6. Only then emit the first candidate from the next layer.

This caused visible pauses in terminal output. The user sees chunks of strings printed, then a long silence while the next layer is being built and sorted.

## Current State

There are now two approaches in the codebase:

- `CandidateEnumerator`: exact ordered mode. It is checkpointable during layer build, but still does not emit from a new layer until that layer is fully built and sorted.
- `DiscoveryCandidateEnumerator`: low-latency mode. It emits each novel neighbor as soon as discovered. This avoids long pauses, but it changes within-layer ordering. It keeps distance layering and deduplicates candidate strings, but it does not preserve likelihood/lexical sorted output.

The remaining research problem is: can we preserve the ordered contract while still streaming smoothly?

## Functional Requirements

A proposed algorithm should ideally:

- Preserve distance-first ordering.
- Preserve likelihood-cost ordering within a distance.
- Preserve lexical ordering within equal distance and equal cost.
- Avoid duplicate emitted candidate strings, or clearly document if dedup is intentionally relaxed.
- If dedup is preserved, handle candidates reachable by multiple paths, preferably using best cost.
- Support min/max distance bands, including `min_distance = 0`.
- Preserve Unicode correctness using char-based indexing.
- Respect enabled/disabled edit operations.
- Handle keyboard-neighbor costs correctly.
- Support large alphabets, for example letters, numbers, symbols.
- Be deterministic across runs.

## Non-Functional Requirements

The solution must be computationally reasonable. It cannot preserve sorting by scanning an exponentially huge unrelated search space.

For example, with seed `"patter"`:

- seed length `n = 6`
- alphabet size around `A ~= 91`
- one-edit branching factor is roughly:
  - deletes: `6`
  - inserts: `7 * 91 = 637`
  - replacements: `6 * 90 = 540`
  - swaps: `5`
  - total: about `1,188` raw neighbors

Generating distance 2 by BFS expansion is roughly on the order of:

```text
~1.2k parents * ~1.3k neighbors ~= 1.5M neighbor events
```

That is large but feasible.

A naive lexicographic scan is not feasible. If we scan all strings of lengths `4..8` over alphabet size `91` and run a DP edit-distance/cost check on each, we scan roughly:

```text
91^4 + 91^5 + 91^6 + 91^7 + 91^8 ~= 4.8e15 strings
```

Even at an unrealistic 1 billion checks/sec, that is around 55 days before overhead. This is catastrophic and unacceptable.

## The Streaming Tension

The hard part is that exact sorting requires global knowledge.

To emit the next candidate under `(distance, cost, lexical)` ordering, the algorithm must know there is no unseen candidate in the same distance layer with:

- lower cost, or
- same cost and lexically earlier string.

The current exact algorithm gets that knowledge by materializing the whole layer. That is correct but causes long pauses.

Discovery-order streaming avoids the pause by emitting immediately, but then it cannot know whether a later parent will produce a cheaper or lexically earlier candidate. So exact sorting is lost.

## Known Candidate Approaches

### Cost-Bucketed Layer Generation

Process distance `d`, group candidates by accumulated cost, and for each cost bucket collect all candidates in that bucket, sort lexically, emit, then proceed.

This preserves ordering if implemented correctly.

Problem: buckets grow with distance and can still become very large. Pauses shrink from "whole layer" to "one bucket," but may still grow badly.

### Priority Queue / Best-First Expansion

Use a heap ordered by `(distance, cost, lexical)` and try to emit the next globally smallest candidate without full layer materialization.

The hard part is dedup and best-cost correctness. A candidate discovered now may later be found through a cheaper path unless the algorithm has a proof that cannot happen.

This needs careful invariants, possibly similar to Dijkstra/A* over string states, plus distance bounds and lexical tie handling.

### Lexicographic Generation With Pruning

Generate candidate strings in lexical order, but only inside a tightly constrained language of strings reachable at distance `d` and cost `c`.

The algorithm must avoid scanning all `A^length` strings. It would need a finite-state or dynamic-programming generator that can jump directly to valid completions.

This may be promising if it can enumerate only valid candidates, or near-valid candidates, not the whole string universe.

### Dynamic-Programming Automaton Approach

Build an automaton or transducer for strings reachable from the seed with exact edit distance and cost. Traverse the automaton in cost/lexical order and emit accepted strings.

This must avoid exponential state blowup and support insert/delete/replace/swap plus custom costs.

This is likely the most promising "novel algorithm" direction worth researching.

## Desired Outcome

Find or design an algorithm that gives reasonable streaming while preserving sorting.

"Reasonable streaming" means:

- no long layer-sized silent pauses,
- no exponentially worse scan over all possible strings,
- time between emitted candidates should be bounded by local work or small frontier/bucket work,
- total work should be comparable to generating the reachable neighborhood, not the entire alphabetic string space.

The agent should explicitly analyze complexity in terms of:

- seed length `n`,
- alphabet size `A`,
- max distance `D`,
- number of reachable unique candidates `U_d`,
- branching factor `B`,
- cost range / number of cost buckets,
- memory required for dedup or frontier state.

The central research question is:

Can we preserve `(distance, likelihood cost, lexical)` order and still emit candidates incrementally, with total computation closer to BFS neighborhood generation than to exhaustive lexicographic scanning?
