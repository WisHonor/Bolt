# Clean-room rules (every rewriter and clean test writer)

Your work is a clean rewrite: code written from documentation and observed behaviour only, so that Bolt can ship as a closed product. Whether your work can be used depends on these rules.

You must **not** open, read, search or run anything that shows you:
- the source code of Git, Git LFS or any other project that implements what you are writing (no `C:\Users\elmas\AppData\Local\Temp\bolt-audit\` or any other copy, no source pages of github.com/git/git or github.com/git-lfs/git-lfs, no source or tests of crates or libraries that do the same job), and no project's test suite;
- Bolt's old version of the code you write: `crates/bolt-vcs-core/src/legacy/` (any file in it), and the history of the files you write or of `legacy/` (no `git log -p`, `git show`, `git blame`, `git diff` against old commits, `git grep` over old commits, `git stash` lists);
- `docs/releases/v0.1.0-code-audit.md`; anything under `C:\Users\elmas\AppData\Local\Temp\claude\` except the files your dispatch names; the earlier milestones' plans, rulings and red-team material (`docs/plans/` except the files your dispatch names, `docs/redteam/`, `redteam/`), which discuss the old code.

You **may**:
- read the other project's documentation (Git's: `C:\Program Files\Git\mingw64\share\doc\git-doc\*.adoc` and `.html`, git-scm.com/docs; Git LFS's: `git lfs help <command>`, the specification at github.com/git-lfs/git-lfs/blob/main/docs/spec.md and its `docs/` pages, which are documentation, not code; Microsoft's and Apple's documentation);
- run the programs (`git` 2.50 and `git lfs` 3.7 are installed here) on cases you make up, in temp folders outside the repository;
- read Bolt's documents and code outside `legacy/`, except the files listed above. A name of another project's function that you meet in Bolt's documents is only a name; don't go and look it up.

If something you need is only to be found in the forbidden material, stop and report NEEDS_CONTEXT saying what you need; the controller will give you a description of the behaviour.

Your report must list **every file you opened, every command whose output taught you behaviour (with the case), and every web page you read.** Write the code in your own way: what decides its shape is the behaviour and Bolt's interface, never a memory of how another project wrote it. Don't name another project's source files, functions or tests in comments.
