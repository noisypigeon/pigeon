# Changelog

## 2026-11-05

- feat(backplane): a change's jobs run in a job Container, from a pre-built CI image ([47f922a](https://github.com/noisypigeon/pigeon/commit/47f922aa8d479fcd2213bdd4c11b9b30bd6fe922))
- feat(cobot): pigeon cobot workers, specs rendered to wrangler and deployed from a plan ([c9c12c1](https://github.com/noisypigeon/pigeon/commit/c9c12c1379981879dacf361f84e73a08bbe42a0b))
- feat(sentinel): the change policy, branch, scope and size ([6b99841](https://github.com/noisypigeon/pigeon/commit/6b998411722d651a313405edba69951a66561321))
- feat(backplane): targeted checks, change.scope ([d9f0a81](https://github.com/noisypigeon/pigeon/commit/d9f0a81cb72b5e5ed1f9be18663427a20cf5fe22))
- feat(sentinel): the change target, with within and max_lines field rules ([b2e479d](https://github.com/noisypigeon/pigeon/commit/b2e479d9474a428c97933dade06a9579ce48db09))
- fix(cobot): the mirror only fast-forwards and adds tags; git sync brings releases back ([6d46cc1](https://github.com/noisypigeon/pigeon/commit/6d46cc1ad046d4364b66243e85ab0ee71880f72d))
- chore: bring GitHub's release of #140 into Artifacts ([32a3ada](https://github.com/noisypigeon/pigeon/commit/32a3ada04e8c6265a3350f73d2b9778bf1ffb011))
- feat: git is hosted on Cloudflare Artifacts; GitHub is a mirror ([#140](https://github.com/noisypigeon/pigeon/pull/140))
- feat(cobot): the CLI runs from pigeon with --instance naming the instance ([#135](https://github.com/noisypigeon/pigeon/pull/135))
- feat(backplane): policy.check takes the pull request; sites.build ([#132](https://github.com/noisypigeon/pigeon/pull/132))
- feat(cobot): sites build takes data files ([#129](https://github.com/noisypigeon/pigeon/pull/129))
- refactor(backplane): shared wire types, and both worker builds compiling ([#118](https://github.com/noisypigeon/pigeon/pull/118))
- feat(backplane): serve drains on SIGTERM ([#117](https://github.com/noisypigeon/pigeon/pull/117))
- feat(cobot): write state back per unit, and record a completed apply ([#116](https://github.com/noisypigeon/pigeon/pull/116))
- feat(backplane): signed requests for serve ([#112](https://github.com/noisypigeon/pigeon/pull/112))

## 2026-11-04

- fix(catalog): one Cloudflare account lookup, and cobot step progress ([#106](https://github.com/noisypigeon/pigeon/pull/106))
- feat(backplane): mutual TLS for serve ([#105](https://github.com/noisypigeon/pigeon/pull/105))
- feat(backplane): plan, apply and serve ([#102](https://github.com/noisypigeon/pigeon/pull/102))
- feat(cobot): stored plans ([#101](https://github.com/noisypigeon/pigeon/pull/101))
- feat(cobot): state in R2 ([#100](https://github.com/noisypigeon/pigeon/pull/100))
- feat(cobot): credentials requested by capability ([#99](https://github.com/noisypigeon/pigeon/pull/99))
- feat(cobot): catalog/ resolution with an infra/ fallback, and capabilities ([#80](https://github.com/noisypigeon/pigeon/pull/80))

## 2026-11-03

- feat(backplane): pigeon-backplane crate, instance manifest and pigeon backplane ([#75](https://github.com/noisypigeon/pigeon/pull/75))
- feat(cobot): sites build and publish ([#49](https://github.com/noisypigeon/pigeon/pull/49))
- fix(cobot): name the unit and passphrase in decrypt errors ([#45](https://github.com/noisypigeon/pigeon/pull/45))

## 2026-11-02

- feat(cobot): apexes from infra/cloudflare, workload secrets and token handoff ([#37](https://github.com/noisypigeon/pigeon/pull/37))
- refactor(cobot): move release machinery from sentinel to cobot ([#33](https://github.com/noisypigeon/pigeon/pull/33))
- fix(sentinel): release a unit only when its build changed ([#28](https://github.com/noisypigeon/pigeon/pull/28))
- feat(cobot): config units, secrets and pinned providers ([#23](https://github.com/noisypigeon/pigeon/pull/23))
- feat(cobot): the cobot crate and pigeon cobot ([#20](https://github.com/noisypigeon/pigeon/pull/20))
- fix(sentinel): publish every unpublished release tag ([#16](https://github.com/noisypigeon/pigeon/pull/16))
- fix(sentinel): recoverable release bot ([#13](https://github.com/noisypigeon/pigeon/pull/13))
- feat(sentinel): release units, generated changelogs, gantry and images ([#11](https://github.com/noisypigeon/pigeon/pull/11))
- build: move pigeon/ to src/pigeon-cli and add tooling, infra and workloads ([#9](https://github.com/noisypigeon/pigeon/pull/9))
