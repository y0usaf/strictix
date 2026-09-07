# Cognitive complexity

`cognitive-complexity` is a warning enabled by default. It reports a lambda
when its score exceeds **5**, highlighting that lambda without offering a fix.
Disable it with `--disable cognitive-complexity`, `[lint].disabled`, or the
usual `# strictix: disable-next-line=cognitive-complexity` directive. The
threshold is currently a constant, like the cyclomatic complexity threshold.

## Nix scoring

This is a Nix adaptation, not an exact implementation of Biome's JavaScript
rule or the full Sonar algorithm. Scores start at zero.

| Construct | Cost |
| --- | --- |
| `if` | 1 plus enclosing conditional depth |
| Final `else` | 1 |
| Direct `else if` | 1; stays at the preceding `if`'s depth |
| Recognized conditional helper call | 1 plus enclosing conditional depth |
| Sequence of `&&`, `||`, or `->` | 1 per consecutive run of the same operator |
| Nested lambda | Scored independently, starting at depth zero |
| Other syntax | No intrinsic cost; its children are still inspected |

Conditions and branches increase conditional depth for their children.
Fully applied `lib.mkIf`, `lib.optional`, `lib.optionals`, `lib.optionalAttrs`,
and `lib.optionalString` calls add 1 plus conditional depth. Both arguments
are inspected at the next depth; there is no extra point for an implicit
empty alternative. For example, three nested helper calls score 1 + 2 + 3 = 6.
Parentheses do not break boolean sequences: `a && (b && c)` scores 1,
while `a && b || c && d` scores 3. Separate boolean expressions, including
those inside negation, are scored independently.

Recognition follows the conventional module argument named `lib` (or an
unbound `lib`), direct local aliases of that library or a helper, sourced
`inherit (lib)`, and names supplied by `with lib;`. A locally defined library
or helper lookalike is ignored. This is a convention-based heuristic: the
linter does not evaluate imports or prove the runtime identity of a module's
`lib` argument. Dynamic selections, custom wrappers, partially applied aliases,
and aliases deeper than 32 resolution steps are outside recognition. An
unknown inner `with` is treated conservatively. A partial call has no helper
cost; a call with extra arguments counts its fully applied inner call once.
`mkMerge` itself has no conditional cost, but helpers inside it are visited.

Attribute sets, `let`, `with`, `assert`, attribute existence tests, and
attribute `or` defaults add no intrinsic cost. Recursion is not scored.
Formal parameter defaults belong to their lambda. Curried functions such as
`x: y: body` are measured as separate lambdas, without double-counting `body`
or adding nesting for each argument. Non-lambda top-level expressions are
outside this rule's scope, matching the cyclomatic rule.

For example, two nested conditionals score 5 and are allowed:

```nix
x: if a then (if b then 1 else 2) else 3
```

Three nested conditionals score 9 and are reported:

```nix
x: if a then (if b then (if c then 1 else 2) else 3) else 4
```

A flat three-condition `else if` chain scores only 4.

## Default calibration on finix

Measured on 2026-09-06 against `/home/y0usaf/finix`, using the 254 files
returned by `strictix check /home/y0usaf/finix --format json`. A temporary
profiling binary parsed the same files and called
`cognitive_complexity(&model, lambda)` for every lambda, including zero
scores. No files produced parse errors. These are the measurements before
refactoring finix to meet the maximum of 5.
The profiler was removed after measurement; the scoring function remains
available in `strictix_lints::cognitive_complexity` for repeating the analysis.

| Score | Lambdas |
| --- | ---: |
| 0 | 238 |
| 1 | 114 |
| 2 | 10 |
| 3 | 16 |
| 4 | 7 |
| 5 | 4 |
| 6 | 2 |
| 7 | 1 |
| 8 | 1 |
| 9 | 1 |
| 10 | 1 |
| 12 | 1 |
| Total | 396 |

| Candidate maximum | Findings (score greater than maximum) |
| --- | ---: |
| 3 | 18 |
| **5** | **7** |
| 6 | 5 |
| 7 | 4 |
| 8 | 3 |
| 10 | 1 |
| 12 | 0 |
| 15 | 0 |

The chosen maximum is **5**, as requested for a stricter baseline. At the
original snapshot it reported seven lambdas. A maximum of 7 would have
reported four; lowering the maximum includes straightforward but branch-heavy
configuration code as candidates for simplification.

| Original file and lambda line | Original score |
| --- | ---: |
| `modules/desktop/session/ui/tomoe/config.nix:1` | 12 |
| `modules/desktop/apps/bolo.nix:1` | 10 |
| `modules/desktop/apps/discord/stable.nix:1` | 9 |
| `modules/dev/ai/omp/default.nix:17` (`ruleFile`) | 8 |
| `modules/dev/ai/opencode/opencode.nix:1` | 7 |
| `modules/dev/ai/paseo/service.nix:10` | 6 |
| `modules/dev/ai/prompts/mapping.nix:5` | 6 |

After refactoring those seven files to meet the maximum of 5, the corpus has
413 lambdas, all scoring at most 5. The highest score in each changed file is:

| File | Highest score after refactoring |
| --- | ---: |
| Tomoe configuration | 3 |
| Bolo | 4 |
| Discord | 3 |
| OMP | 3 |
| OpenCode | 3 |
| Paseo | 2 |
| Skill-directory mapping | 0 |

The refactors extract domain-specific rendering and configuration functions,
replace repeated optional-field merges with filtering, and select layouts
from a table. They do not suppress diagnostics or disguise conditional
helpers with wrapper aliases.

The original syntax-only measurement ranged from 0 to 4. It omitted the
conditional helper calls that carry much of this corpus's logic; those
numbers were insufficient for calibration. Counting the helper calls is
what reveals these findings, rather than lowering the threshold until
something happens to trigger. The score-4 Lua key formatter remains allowed.

This is an initial default based on one codebase, not a universally optimal
threshold. Large declarations, arbitrary function calls, and embedded Lua or
shell control flow are not scored; Nix interpolation expressions are scored.
Tests cover helper nesting, alias resolution, shadowing, the exact reporting
boundary, and suppression.

For comparison, [Biome's cognitive complexity rule](https://biomejs.dev/linter/rules/no-excessive-cognitive-complexity/javascript/)
defaults to 15. Numerical scores are not directly comparable across these
implementations.
