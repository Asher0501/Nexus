# Code Review Task

You are reviewing Rust source code in the Nexus engine. Use Read/Write/Edit tools.

## Step 1: Read the source files
Read these files under engine/crates/engine/src/:
- graph/validator.rs
- graph/builder.rs
- graph/scheduler.rs
- runtime/engine.rs
- nodeshell/subprocess.rs
- nodeshell/llm.rs
- nodeshell/llm_sdk.rs
- model/provider.rs
- model/workflow.rs

## Step 2: Read audit trail
Read review_doc.md to see previous rounds' findings (may not exist on round 1).

## Step 3: Review across 4 dimensions

### BUGS
- Logic errors, off-by-one, incorrect conditions
- Missing error handling, unwrap() on Option/Result
- Race conditions, deadlocks in async code
- Incorrect state transitions

### SECURITY
- Unsafe blocks without safety comments
- Command injection risks in shell/subprocess
- Path traversal in file operations
- Missing input validation

### PERFORMANCE
- Unnecessary clones or allocations
- Blocking operations in async context
- Inefficient data structures
- Redundant work in hot paths

### CODE QUALITY
- Unused imports, variables, dead code
- Inconsistent naming or patterns
- Missing documentation on public APIs
- Overly complex functions

## Step 4: Write findings to review_doc.md
Format each issue:
```
## Review Round N

### FILE: path/to/file.rs
- **[SEVERITY]** **[CATEGORY]** Issue description
  - LINE: line number or range
  - FIND: relevant code snippet
  - SUGGESTION: how to fix
```
Separate rounds with `---`.

## Step 5: Decision
When ALL steps above are complete, decide:
- Issues found → needs_fix
- All clean → approved
