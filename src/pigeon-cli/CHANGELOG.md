# Changelog

## 2026-11-04

- feat(cobot): catalog/ resolution with an infra/ fallback, and capabilities ([#80](https://github.com/noisypigeon/pigeon/pull/80))
- feat(sentinel): a fields rule can forbid extra fields ([#79](https://github.com/noisypigeon/pigeon/pull/79))

## 2026-11-03

- feat(backplane): pigeon-backplane crate, instance manifest and pigeon backplane ([#75](https://github.com/noisypigeon/pigeon/pull/75))
- feat(cobot): agnostic modules and each blocks ([#67](https://github.com/noisypigeon/pigeon/pull/67))
- refactor(cobot): only the site config contract ([#60](https://github.com/noisypigeon/pigeon/pull/60))
- feat(cobot): build static sites from config.yaml ([#58](https://github.com/noisypigeon/pigeon/pull/58))
- feat(static-site-template): a site config contract and dated posts ([#57](https://github.com/noisypigeon/pigeon/pull/57))
- feat(cobot): release a pigeon-cobot image ([#53](https://github.com/noisypigeon/pigeon/pull/53))
- fix(cobot): the apex token's transform rules permission ([#51](https://github.com/noisypigeon/pigeon/pull/51))
- feat(cobot): sites build and publish ([#49](https://github.com/noisypigeon/pigeon/pull/49))
- feat(infra): the apex's site rulesets and the account outputs ([#48](https://github.com/noisypigeon/pigeon/pull/48))
- feat(cobot): static-site workloads on R2 ([#47](https://github.com/noisypigeon/pigeon/pull/47))
- fix(cobot): name the unit and passphrase in decrypt errors ([#45](https://github.com/noisypigeon/pigeon/pull/45))
- refactor(cobot): drop the all-zones apex ([#43](https://github.com/noisypigeon/pigeon/pull/43))
- refactor(workloads): move the noisypigeon.com apex to its zone ([#42](https://github.com/noisypigeon/pigeon/pull/42))
- feat(cobot): per-zone apexes and unit moves ([#41](https://github.com/noisypigeon/pigeon/pull/41))

## 2026-11-02

- feat(cobot): apexes from infra/cloudflare, workload secrets and token handoff ([#37](https://github.com/noisypigeon/pigeon/pull/37))
- refactor(cobot): move release machinery from sentinel to cobot ([#33](https://github.com/noisypigeon/pigeon/pull/33))
- feat(infra): cloudflare provisioner unit ([#30](https://github.com/noisypigeon/pigeon/pull/30))
- fix(sentinel): release a unit only when its build changed ([#28](https://github.com/noisypigeon/pigeon/pull/28))
- fix(cobot): copy only the lockfile blocks a unit uses ([#27](https://github.com/noisypigeon/pigeon/pull/27))
- feat(cobot): config units, secrets and pinned providers ([#23](https://github.com/noisypigeon/pigeon/pull/23))
- feat(cobot): plan on PR, apply on merge ([#21](https://github.com/noisypigeon/pigeon/pull/21))
- feat(cobot): the cobot crate and pigeon cobot ([#20](https://github.com/noisypigeon/pigeon/pull/20))
- feat(sentinel): find rule and optional release images ([#19](https://github.com/noisypigeon/pigeon/pull/19))
- fix(sentinel): publish every unpublished release tag ([#16](https://github.com/noisypigeon/pigeon/pull/16))
- fix(sentinel): recoverable release bot ([#13](https://github.com/noisypigeon/pigeon/pull/13))
- feat(sentinel): release units, generated changelogs, gantry and images ([#11](https://github.com/noisypigeon/pigeon/pull/11))
- build: move pigeon/ to src/pigeon-cli and add tooling, infra and workloads ([#9](https://github.com/noisypigeon/pigeon/pull/9))
