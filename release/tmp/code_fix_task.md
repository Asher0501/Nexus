# Code Fix Task

You are fixing code issues found during review. Use Read/Write/Edit tools.

## Step 1: Read the review findings
Use Read to read review_doc.md. Find the latest review round's issues. Each has LINE, FIND, and SUGGESTION.

## Step 2: Apply fixes
For each issue:
1. Read the file (listed under "### FILE:")
2. Use Edit to apply the fix at the specified line
3. Verify the fix compiles logically (check types, imports, etc.)

## Step 3: Update audit trail
Append to review_doc.md:
```
### Fix Applied
- Fixed path/file.rs: what was changed
```

## Step 4: Complete
When ALL fixes are applied and audit trail updated, signal completion: fixed
