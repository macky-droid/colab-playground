# Changelog

## [0.3.0](https://github.com/scarletkc/colab-playground/compare/v0.2.0...v0.3.0) (2026-11-19)


### Added

* download, check and install updates from inside the app ([#41](https://github.com/scarletkc/colab-playground/issues/41)) ([cfd98c2](https://github.com/scarletkc/colab-playground/commit/cfd98c2e6781e97a42cca35fa976d30a27f51a70))


### Changed

* name the downloads without the version ([cfd98c2](https://github.com/scarletkc/colab-playground/commit/cfd98c2e6781e97a42cca35fa976d30a27f51a70))


### Fixed

* **core:** end a session whose runtime was never recorded once nothing running could be it ([#38](https://github.com/scarletkc/colab-playground/issues/38)) ([9e5eb7e](https://github.com/scarletkc/colab-playground/commit/9e5eb7e34b7b1f8eb4800e66a1cf6243bf0ea6f7))

## [0.2.0](https://github.com/scarletkc/colab-playground/compare/v0.1.0...v0.2.0) (2026-11-18)


### Added

* **app:** add Japanese and Korean as display languages ([#19](https://github.com/scarletkc/colab-playground/issues/19)) ([def0fa8](https://github.com/scarletkc/colab-playground/commit/def0fa8049c636d51400b47e9a421c785c451a60))
* **app:** say how long Colab has not answered a request for a runtime ([#24](https://github.com/scarletkc/colab-playground/issues/24)) ([0f6c5f0](https://github.com/scarletkc/colab-playground/commit/0f6c5f029fb7d2f39e0433379d1db9987cbdd4f4))


### Changed

* **app:** show no app mark or name beside the traffic lights on macOS ([#11](https://github.com/scarletkc/colab-playground/issues/11)) ([2583758](https://github.com/scarletkc/colab-playground/commit/2583758f3934a00ad5d58ed42e4c00aa8664c0f4))


### Fixed

* **app:** give the New session and Forward dialogs a scrollbar and the window's height ([#18](https://github.com/scarletkc/colab-playground/issues/18)) ([72e6009](https://github.com/scarletkc/colab-playground/commit/72e60097f291a0a08be7735c86dfee6894c494e6))
* **app:** keep the billing title when only some accounts have unknown runtime ownership ([#31](https://github.com/scarletkc/colab-playground/issues/31)) ([17a099d](https://github.com/scarletkc/colab-playground/commit/17a099d1f7555b6e3519e511aabc2034b3350e19))
* **app:** list plugins before the Colab CLI is installed ([#10](https://github.com/scarletkc/colab-playground/issues/10)) ([778c5f9](https://github.com/scarletkc/colab-playground/commit/778c5f94caba6602023a876316ee5684f073472c))
* **app:** mark the unmanaged-runtime warning as a last reading when usage is stale ([#26](https://github.com/scarletkc/colab-playground/issues/26)) ([bc23ecd](https://github.com/scarletkc/colab-playground/commit/bc23ecd6e11ce265ecad2ee62b91ca0857deb3a9))
* **app:** say when the runtimes left behind by an account switch are unknown ([#20](https://github.com/scarletkc/colab-playground/issues/20)) ([2cdc26c](https://github.com/scarletkc/colab-playground/commit/2cdc26c8cfebc7391de54cfe38ed65b4ac3ccf7a))
* **core:** keep WSL and its session bus up so the keyring is reachable and stays unlocked ([#16](https://github.com/scarletkc/colab-playground/issues/16)) ([d7a0386](https://github.com/scarletkc/colab-playground/commit/d7a0386e80a913feb6c0a4baa87a2e4c6a187caf))
* **core:** let Cancel and Stop all end a runtime request Colab never answers ([#21](https://github.com/scarletkc/colab-playground/issues/21)) ([586e26a](https://github.com/scarletkc/colab-playground/commit/586e26a66cd57e6d843027b99b20b63a83c1c5db))
* **core:** reconcile ended sessions before counting unmanaged runtimes ([#27](https://github.com/scarletkc/colab-playground/issues/27)) ([7048695](https://github.com/scarletkc/colab-playground/commit/704869572c0f9249897eabb613e5f79796c2640a))
* **core:** show wsl.exe messages legibly and stop treating them as a WSL distribution ([#22](https://github.com/scarletkc/colab-playground/issues/22)) ([91b139a](https://github.com/scarletkc/colab-playground/commit/91b139ae092846d00a9364dc27fa1c644904598e)), closes [#14](https://github.com/scarletkc/colab-playground/issues/14)

## 0.1.0 (2026-11-17)


### Added

* initial release ([6701a59](https://github.com/scarletkc/colab-playground/commit/6701a59889884510a97f4e17ae7b03a5416955f5))
