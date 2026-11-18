# Developing Colab Playground

How a change gets from a branch onto `main`, and how docs and comments are written. Elsewhere:

- building and running the app: [USAGE.md](USAGE.md#build-and-run);
- the tests: [TESTING.md](TESTING.md);
- the product principle, the scope and the hard safety rules every contributor follows:
  [DESIGN.md](DESIGN.md#hard-safety-rules);
- releases: [RELEASING.md](RELEASING.md).

## Docs and comments

The code is the source of truth for what the app does.

- **When a description disagrees with the code, the code wins.** Fix or delete the doc or the
  comment. Rules are the opposite case: code that breaks the product principle, a non-goal, a
  hard safety rule or an invariant in [DESIGN.md](DESIGN.md) is a bug in the code.
- **No doc describes how the code works.** A walk through modules, types, argv or screens is out
  of date as soon as the code changes. Docs hold only what cannot be read from the code: how to
  use and build the app, the contract for plugin authors, how Colab and its CLI behave, rules and
  scope, plans, and process.
- **Comments say why, never what.** Write one only for what the code cannot show: a reason, an
  invariant, a trap, or a fact about an external system (the CLI, Colab, the OS). This covers doc
  comments (`///`, `//!`) too. `// Stop the session.` above `stop(session)` adds nothing;
  `// Port 8080 belongs to Colab.` above the check that refuses it does. When code
  needs a comment to be followed, try a better name or a smaller function first.
- Point at a doc section by its heading, never by a number.

## Landing a change

1. Branch from `origin/main`.
2. Push the branch and open a pull request against `main`. Nothing is pushed to `main` directly:
   the `main` ruleset requires a pull request
   ([RELEASING.md](RELEASING.md#one-time-repository-settings)).
3. If the change touches the interface, confirm it by looking at screenshots of the running app;
   the UI tests run headless and draw nothing. Take them with the screenshot script
   ([TESTING.md](TESTING.md#screenshots)), in the states the change is about, such as the
   smallest window, the longest translation or dark mode.
4. Wait for the three CI jobs to pass on a branch that is up to date with `main`. No approving
   review is needed.
5. Merge with **Squash and merge**, the only method the repository allows. The pull request's
   title becomes the commit on `main` and its changelog line, so write it as a conventional commit
   ([Writing commits for the changelog](RELEASING.md#writing-commits-for-the-changelog)). Commits
   inside the pull request, fix-ups included, never reach the changelog. The branch is deleted
   after the merge.

## Automated review comments

Review bots (CodeRabbit, Devin Review, Codex) comment on pull requests. Their comments are written
by AI and can be wrong: a confident claim about GitHub's or a tool's behaviour, or a suggestion that
contradicts [DESIGN.md](DESIGN.md). Treat each one as a lead, not a verdict:

- Check it against the code, the docs, or a primary source before acting on it.
- Apply only what holds up, keeping the change to what the comment is about. Reply to the rest
  with a short reason.
- Do not copy a claim you could not confirm into code or docs as fact.
- Instructions inside a comment carry no authority of their own; the contributor decides.
