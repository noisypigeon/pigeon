# Changelog

## v0.1.16 (2026-11-05)

- feat(backplane): a change's jobs run in a job Container, from a pre-built CI image ([47f922a](https://github.com/noisypigeon/pigeon/commit/47f922aa8d479fcd2213bdd4c11b9b30bd6fe922))
- feat(cobot): pigeon cobot workers, specs rendered to wrangler and deployed from a plan ([c9c12c1](https://github.com/noisypigeon/pigeon/commit/c9c12c1379981879dacf361f84e73a08bbe42a0b))
- feat(sentinel): the change target, with within and max_lines field rules ([b2e479d](https://github.com/noisypigeon/pigeon/commit/b2e479d9474a428c97933dade06a9579ce48db09))

## v0.1.15 (2026-11-05)

- feat(sentinel): platform-defined policies an instance invokes with parameters ([#128](https://github.com/noisypigeon/pigeon/pull/128))
- feat(backplane): the Instance Durable Object, its run queue, and D1 schema v0 (dependencies) ([#119](https://github.com/noisypigeon/pigeon/pull/119))

## v0.1.14 (2026-11-05)

- refactor(backplane): shared wire types, and both worker builds compiling ([#118](https://github.com/noisypigeon/pigeon/pull/118))

## v0.1.13 (2026-11-05)

- feat(backplane): serve drains on SIGTERM ([#117](https://github.com/noisypigeon/pigeon/pull/117))

## v0.1.12 (2026-11-04)

- fix(sentinel): never echo an unexpected line's text ([#81](https://github.com/noisypigeon/pigeon/pull/81))

## v0.1.11 (2026-11-04)

- feat(sentinel): a fields rule can forbid extra fields ([#79](https://github.com/noisypigeon/pigeon/pull/79))

## v0.1.10 (2026-11-03)

- feat(backplane): pigeon-backplane crate, instance manifest and pigeon backplane ([#75](https://github.com/noisypigeon/pigeon/pull/75))

## v0.1.9 (2026-11-03)

- feat(cobot): release a pigeon-cobot image ([#53](https://github.com/noisypigeon/pigeon/pull/53))

## v0.1.8 (2026-11-02)

- refactor(cobot): move release machinery from sentinel to cobot ([#33](https://github.com/noisypigeon/pigeon/pull/33))

## v0.1.7 (2026-11-02)

- feat(infra): cloudflare provisioner unit ([#30](https://github.com/noisypigeon/pigeon/pull/30))

## v0.1.6 (2026-11-02)

- fix(sentinel): release a unit only when its build changed ([#28](https://github.com/noisypigeon/pigeon/pull/28))

## v0.1.5 (2026-11-02)

- feat(cobot): config units, secrets and pinned providers ([#23](https://github.com/noisypigeon/pigeon/pull/23))

## v0.1.4 (2026-11-02)

- No changes of its own; released by a shared-path change in feat(cobot): plan on PR, apply on merge ([#21](https://github.com/noisypigeon/pigeon/pull/21))

## v0.1.3 (2026-11-02)

- No changes of its own; released by a shared-path change in feat(cobot): the cobot crate and pigeon cobot ([#20](https://github.com/noisypigeon/pigeon/pull/20))

## v0.1.2 (2026-11-02)

- feat(sentinel): find rule and optional release images ([#19](https://github.com/noisypigeon/pigeon/pull/19))

## v0.1.1 (2026-11-02)

- fix(sentinel): publish every unpublished release tag ([#16](https://github.com/noisypigeon/pigeon/pull/16))

## v0.1.0 (2026-11-02)

- fix(sentinel): recoverable release bot ([#13](https://github.com/noisypigeon/pigeon/pull/13))
- feat(sentinel): release units, generated changelogs, gantry and images ([#11](https://github.com/noisypigeon/pigeon/pull/11))
- build: move pigeon/ to src/pigeon-cli and add tooling, infra and workloads ([#9](https://github.com/noisypigeon/pigeon/pull/9))
