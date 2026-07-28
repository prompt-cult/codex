## Kata S6 -- Fetch and summarize service health

Use the agent-dsl tool to call the registered health, metrics-summary,
and deployment-status endpoints in parallel. Join the validated
responses and ask an operations agent to produce a current health
summary. Fail if any required endpoint or schema is not registered; do
not construct arbitrary URLs.
