"""Locate the Agent Workspace image repository (github.com/fliaping/agent-workspace).

A few tests drive this repository's binaries with scripts that ship in the
workspace image. Point AGENT_WORKSPACE_SRC at a checkout of that repository, or
check it out next to this one; without it those tests are skipped.
"""
import os
from pathlib import Path

IMAGE = Path(os.environ.get('AGENT_WORKSPACE_SRC', Path(__file__).resolve().parents[2] / 'agent-workspace'))
AVAILABLE = (IMAGE / 'scripts').is_dir()
REASON = 'needs the Agent Workspace repository: set AGENT_WORKSPACE_SRC or check it out next to cranibox'
