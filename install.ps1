# The Agent Workspace container installer moved to github.com/fliaping/agent-workspace.
# This stub keeps the old one-liner working:
#   irm https://raw.githubusercontent.com/fliaping/cranibox/main/install.ps1 | iex
Write-Host "Agent Workspace moved to https://github.com/fliaping/agent-workspace; running its installer."
Invoke-RestMethod https://raw.githubusercontent.com/fliaping/agent-workspace/main/install.ps1 | Invoke-Expression
