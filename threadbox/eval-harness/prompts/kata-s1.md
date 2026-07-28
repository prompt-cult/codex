## Kata S1 -- Parallel review of the current change

Use the agent-dsl tool to review the current branch without modifying
it. Run a correctness reviewer and a security reviewer in parallel,
join their findings, remove duplicates, and ask a final reporting
agent to rank the remaining issues by severity. Use at most three
agent calls and return a concise Markdown report with file and line
references.
