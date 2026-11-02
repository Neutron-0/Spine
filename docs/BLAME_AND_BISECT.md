# Spine Blame and Bisect Engine

## Overview
`spine blame` and `spine bisect` provide history inspection and automated defect localization.

## `spine blame`

Line provenance attribution for tracked source files.

```bash
spine blame <path> [<rev>]
```

### Attribution Algorithm
1. Locates file blob in `<rev>` (defaults to HEAD).
2. Traverses commit history backwards along first parents.
3. For each commit, inspects the parent commit's version of `<path>`.
4. If lines in the target version do not match lines in the parent version, the difference is attributed to that commit.
5. Formatted output attributes each line to its originating commit hash, author name, and ISO timestamp:
   ```
   a4f541a0 (Charlie         2026-11-02 12:41    1) fn main() {
   71611a8b (Bob             2026-11-02 12:40    2)     println!("Buggy line");
   9d27118a (Alice           2026-11-02 12:39    3) }
   ```

## `spine bisect`

Binary search defect localization over commit history DAGs.

```bash
spine bisect start [<bad> [<good>]]
spine bisect bad [<rev>]
spine bisect good [<rev>]
spine bisect skip [<rev>]
spine bisect status
spine bisect reset
```

### DAG Partitioning Algorithm
1. Collects candidate commits reachable from `<bad>` but not reachable from any `<good>` commit.
2. Filters out commits explicitly marked `<skip>`.
3. Computes the topological midpoint that partitions the search space in half.
4. Checks out the midpoint commit into the working tree.
5. Repeats until only 1 candidate remains, reporting the first bad commit.
6. `spine bisect reset` restores original HEAD prior to bisect session.
