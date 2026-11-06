# Changelog

## v0.1.30 (2026-11-05)

- feat(infra): the control plane's Worker spec, ready for the owner's deploy ([7d013aa](https://github.com/noisypigeon/pigeon/commit/7d013aabcf2d545536400b5a2a189a2602c356ca))
- feat(backplane): a change's jobs run in a job Container, from a pre-built CI image ([47f922a](https://github.com/noisypigeon/pigeon/commit/47f922aa8d479fcd2213bdd4c11b9b30bd6fe922))
- feat(backplane): the Worker serves changes over MCP, behind Access ([49f9dc5](https://github.com/noisypigeon/pigeon/commit/49f9dc507835a4cf03c9d96ac54fe788b1336f6b))
- feat(backplane): change.* operations, the MCP handler and Access checks ([c80edf9](https://github.com/noisypigeon/pigeon/commit/c80edf99727be4bb159cd0bf2e63aec76b31e9e6))
- feat(backplane): the change state machine and its D1 tables ([17a80be](https://github.com/noisypigeon/pigeon/commit/17a80be9f87879a51b8f4b8f01ce4773af2052a9))
- feat(cobot): pigeon cobot workers, specs rendered to wrangler and deployed from a plan ([c9c12c1](https://github.com/noisypigeon/pigeon/commit/c9c12c1379981879dacf361f84e73a08bbe42a0b))
- feat(sentinel): the change policy, branch, scope and size ([6b99841](https://github.com/noisypigeon/pigeon/commit/6b998411722d651a313405edba69951a66561321))
- feat(backplane): targeted checks, change.scope ([d9f0a81](https://github.com/noisypigeon/pigeon/commit/d9f0a81cb72b5e5ed1f9be18663427a20cf5fe22))
- feat(sentinel): the change target, with within and max_lines field rules ([b2e479d](https://github.com/noisypigeon/pigeon/commit/b2e479d9474a428c97933dade06a9579ce48db09))
- fix(cobot): the mirror only fast-forwards and adds tags; git sync brings releases back ([6d46cc1](https://github.com/noisypigeon/pigeon/commit/6d46cc1ad046d4364b66243e85ab0ee71880f72d))
- chore: bring GitHub's release of #140 into Artifacts ([32a3ada](https://github.com/noisypigeon/pigeon/commit/32a3ada04e8c6265a3350f73d2b9778bf1ffb011))

## v0.1.29 (2026-11-05)

- feat: git is hosted on Cloudflare Artifacts; GitHub is a mirror ([#140](https://github.com/noisypigeon/pigeon/pull/140))

## v0.1.28 (2026-11-05)

- feat(cobot): an instance's config.yaml declares units by kind ([#138](https://github.com/noisypigeon/pigeon/pull/138))

## v0.1.27 (2026-11-05)

- fix(cobot): discovery makes both roots absolute; Proton's verification inputs are published ([#137](https://github.com/noisypigeon/pigeon/pull/137))

## v0.1.26 (2026-11-05)

- feat(cobot): the CLI runs from pigeon with --instance naming the instance ([#135](https://github.com/noisypigeon/pigeon/pull/135))

## v0.1.25 (2026-11-05)

- feat(backplane): policy.check takes the pull request; sites.build ([#132](https://github.com/noisypigeon/pigeon/pull/132))

## v0.1.24 (2026-11-05)

- feat(cobot): workloads take the run's own Cloudflare token ([#131](https://github.com/noisypigeon/pigeon/pull/131))

## v0.1.23 (2026-11-05)

- feat(cobot): sites build takes data files ([#129](https://github.com/noisypigeon/pigeon/pull/129))

## v0.1.22 (2026-11-05)

- feat(sentinel): platform-defined policies an instance invokes with parameters ([#128](https://github.com/noisypigeon/pigeon/pull/128))

## v0.1.21 (2026-11-05)

- feat(backplane): architecture.describe and its layout ([#126](https://github.com/noisypigeon/pigeon/pull/126))

## v0.1.20 (2026-11-05)

- refactor(cobot): every site names its template ([#125](https://github.com/noisypigeon/pigeon/pull/125))
- feat(cobot): signed Container dispatch ([#123](https://github.com/noisypigeon/pigeon/pull/123))

## v0.1.19 (2026-11-05)

- feat(cobot): named site templates, and sites without content ([#121](https://github.com/noisypigeon/pigeon/pull/121))

## v0.1.18 (2026-11-05)

- feat(backplane): the Instance Durable Object, its run queue, and D1 schema v0 ([#119](https://github.com/noisypigeon/pigeon/pull/119))

## v0.1.17 (2026-11-05)

- refactor(backplane): shared wire types, and both worker builds compiling ([#118](https://github.com/noisypigeon/pigeon/pull/118))

## v0.1.16 (2026-11-05)

- feat(backplane): serve drains on SIGTERM ([#117](https://github.com/noisypigeon/pigeon/pull/117))

## v0.1.15 (2026-11-05)

- feat(cobot): write state back per unit, and record a completed apply ([#116](https://github.com/noisypigeon/pigeon/pull/116))

## v0.1.14 (2026-11-05)

- feat(backplane): signed requests for serve ([#112](https://github.com/noisypigeon/pigeon/pull/112))

## v0.1.13 (2026-11-04)

- feat(cobot): name the attributes an update changes ([#108](https://github.com/noisypigeon/pigeon/pull/108))

## v0.1.12 (2026-11-04)

- fix(catalog): one Cloudflare account lookup, and cobot step progress ([#106](https://github.com/noisypigeon/pigeon/pull/106))

## v0.1.11 (2026-11-04)

- feat(backplane): mutual TLS for serve ([#105](https://github.com/noisypigeon/pigeon/pull/105))
- feat(tooling): spike TLS checks (dependencies) ([#104](https://github.com/noisypigeon/pigeon/pull/104))

## v0.1.10 (2026-11-04)

- feat(backplane): plan, apply and serve ([#102](https://github.com/noisypigeon/pigeon/pull/102))

## v0.1.9 (2026-11-04)

- feat(cobot): stored plans ([#101](https://github.com/noisypigeon/pigeon/pull/101))

## v0.1.8 (2026-11-04)

- feat(cobot): state in R2 ([#100](https://github.com/noisypigeon/pigeon/pull/100))

## v0.1.7 (2026-11-04)

- feat(cobot): credentials requested by capability ([#99](https://github.com/noisypigeon/pigeon/pull/99))

## v0.1.6 (2026-11-04)

- feat(cobot): catalog module references and retired changelogs ([#94](https://github.com/noisypigeon/pigeon/pull/94))

## v0.1.5 (2026-11-04)

- feat(cobot): local units run only when named ([#91](https://github.com/noisypigeon/pigeon/pull/91))

## v0.1.4 (2026-11-04)

- feat(cobot): a release tier can move with from ([#84](https://github.com/noisypigeon/pigeon/pull/84))

## v0.1.3 (2026-11-04)

- fix(sentinel): never echo an unexpected line's text ([#81](https://github.com/noisypigeon/pigeon/pull/81))

## v0.1.2 (2026-11-04)

- feat(cobot): catalog/ resolution with an infra/ fallback, and capabilities ([#80](https://github.com/noisypigeon/pigeon/pull/80))

## v0.1.1 (2026-11-04)

- feat(sentinel): a fields rule can forbid extra fields ([#79](https://github.com/noisypigeon/pigeon/pull/79))

## v0.1.0 (2026-11-03)

- feat(backplane): pigeon-backplane crate, instance manifest and pigeon backplane ([#75](https://github.com/noisypigeon/pigeon/pull/75))
