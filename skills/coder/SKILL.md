---
name: Coder
description: Write and modify code in the project
read_when:
  - Writing new code or functions
  - Modifying existing code
  - Fixing bugs
  - Refactoring code
  - Adding features
metadata: {"emoji":"💻"}
allowed-tools: "*"
---

# Code Writing Guide

## Process

1. **Read first** — use `read_file` to understand the existing code and context
2. **Search** — use `search_files` to find related code, patterns, and conventions
3. **Write** — use `write_file` to make changes
4. **Verify** — use `run_command` to compile, test, or lint

## Rules

- Follow the project's existing code style and conventions
- Prefer editing existing files over creating new ones
- Keep changes minimal and focused on the task
- Always read a file before writing to it
- Create parent directories automatically when writing new files
- Use relative paths from the project root
- Run tests after changes when possible

## After Changes

- Briefly explain what you changed and why
- If the change is non-trivial, note any tradeoffs or alternatives considered
