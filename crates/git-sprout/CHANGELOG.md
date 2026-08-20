# Changelog

## 0.1.0 (2026-08-20)


### Features

* materialise the new worktree with block clones ([0da8c3a](https://github.com/alltuner/git-sprout/commit/0da8c3af6081b170522be61558f1af9507a2e331))
* parse the git worktree add surface and delegate everything ([e6e28c1](https://github.com/alltuner/git-sprout/commit/e6e28c1f8a5636be1f16affde127e776430cbb9b))
* report how many subtrees were cloned in one call ([a9fef63](https://github.com/alltuner/git-sprout/commit/a9fef6322182a343b242d275c427964c9570b170))
* scaffold the git-sprout workspace and release pipeline ([bd4ae0f](https://github.com/alltuner/git-sprout/commit/bd4ae0f6c6723c6eeab55165d40e82a0133ab3f1))
* the tool — argument parsing, block cloning and index-based verification ([5345014](https://github.com/alltuner/git-sprout/commit/5345014daefd95d8e5ca76d219a500c2f35baf53))


### Bug Fixes

* build and test on linux and windows as well as macos ([52ddf14](https://github.com/alltuner/git-sprout/commit/52ddf14431ab3f1bb4996a50a39848d93a8fe683))
* delegate on an unborn HEAD and give clones a checkout's permissions ([21199b4](https://github.com/alltuner/git-sprout/commit/21199b4fa2b14023d458f25d947911fa05c8c4d0))
* fold collision groups over the source's paths as well as the target's ([a6de732](https://github.com/alltuner/git-sprout/commit/a6de7325c347b5aa898ed71567490e02074cda7b))
* keep the index version the repository asked for ([fe3cb22](https://github.com/alltuner/git-sprout/commit/fe3cb220b152357b790c8790c60a3a5561f22339))
* leave a split-index repository to git ([d942b77](https://github.com/alltuner/git-sprout/commit/d942b7727bf7457078298c1140986142eb4d95a7))
* settle case collisions like git, and survive an interrupt ([f4d7553](https://github.com/alltuner/git-sprout/commit/f4d75535ef4193fcc62370e12243908dd3e6f8e8))
* stop deadlocking against git on large path lists ([49d5767](https://github.com/alltuner/git-sprout/commit/49d57671bc5696548afea5e23995f6016fcb7b16))


### Documentation Updates

* record where the build spec's verification rule is unsound ([93c1c1a](https://github.com/alltuner/git-sprout/commit/93c1c1a6c211ce42608571f4e20c9d5ae5879301))


### Styling Changes

* satisfy clippy on the collision regression test ([02dff94](https://github.com/alltuner/git-sprout/commit/02dff945b99a8e9194760f0352321fcb87717799))
