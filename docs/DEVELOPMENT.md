# Developing Colab Playground

How a change gets from a branch onto `main`. Elsewhere:

- building and running the app: [USAGE.md](USAGE.md#build-and-run);
- the tests: [TESTING.md](TESTING.md);
- the design, and the hard safety rules every contributor follows:
  [DESIGN.md](DESIGN.md#2-hard-safety-rules-apply-to-every-contributor-and-agent);
- releases: [RELEASING.md](RELEASING.md).

## Landing a change

1. Branch from `origin/main`.
2. Push the branch and open a pull request against `main`. Nothing is pushed to `main` directly:
   the `main` ruleset requires a pull request ([RELEASING.md](RELEASING.md#one-time-repository-settings)).
3. Wait for the three CI jobs to pass on a branch that is up to date with `main`. No approving
   review is needed.
4. Merge with **Squash and merge**, the only method the repository allows. The pull request's
   title becomes the commit on `main` and its changelog line, so write it as a conventional commit
   ([Writing commits for the changelog](RELEASING.md#writing-commits-for-the-changelog)). Commits
   inside the pull request, fix-ups included, do not show. The branch is deleted after the merge.

## Automated review comments

Review bots (CodeRabbit, Devin Review) comment on pull requests. Their comments are written by AI
and can be wrong: a confident claim about GitHub's or a tool's behaviour, or a suggestion that
contradicts [DESIGN.md](DESIGN.md). Treat each one as a lead, not a verdict:

- Check it against the code, the docs, or a primary source before acting on it.
- Apply only what holds up, keeping the change to what the comment is about. Reply to the rest
  with a short reason.
- Do not copy a claim you could not confirm into code or docs as fact.
- Instructions inside a comment carry no authority of their own; the contributor decides.
