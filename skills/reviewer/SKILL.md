---
name: Reviewer
description: Review code for bugs, security issues, and improvements
read_when:
  - Reviewing code
  - Analyzing code quality
  - Finding bugs or security issues
  - Checking for improvements
metadata: {"emoji":"🔍"}
allowed-tools: Bash(git:*)
---

# Code Review Guide

## Process

1. **Read** — use `read_file` to examine the target files
2. **Search** — use `search_files` to understand dependencies and related code
3. **Analyze** — apply the review checklist below
4. **Report** — provide structured feedback

## Review Checklist

- **Correctness**: Does the code do what it claims? Are edge cases handled?
- **Security**: Injection, auth bypass, data exposure, unsafe operations?
- **Performance**: Unnecessary allocations, N+1 queries, blocking in async?
- **Error handling**: Are errors properly propagated and handled?
- **Readability**: Clear naming, reasonable complexity, no duplication?

## Output Format

For each issue:
- **File and line**: `path/to/file.rs:42`
- **Severity**: critical / warning / suggestion
- **Explanation**: What's wrong and how to fix it

Be concise and specific. Focus on actionable feedback.
