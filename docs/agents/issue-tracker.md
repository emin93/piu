# Issue tracker

Più uses GitHub Issues in `emin93/piu`.

Use `gh issue list`, `gh issue view <number> --comments`, and `gh issue create` from the repository. The originating product specification is the parent issue. Implementation tickets reference it under `## Parent` and list every blocking issue under `## Blocked by`.

The triage label is `ready-for-agent`. It means the issue is specified, independently implementable, and may be claimed when every blocker is closed. Agents update issue bodies only to correct the contract; implementation progress belongs in commits and pull requests.

Do not close the parent specification issue. Close an implementation ticket only after its acceptance criteria pass on the integrated `main` branch.
