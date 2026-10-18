# The xz project's test files

These are the test files of XZ Utils, its `tests/files` folder, copied unchanged as fixtures of the archive crate's xz tests (`crates/diroxide-archive/tests/it/xz_project.rs`). They are data the tests read, never code that runs.

## Where they come from

- Repository: https://github.com/tukaani-project/xz, folder `tests/files`.
- Tag: `v5.8.4`, the latest release tag when they were fetched.
- Commit: `d3e650e63c110e830fd5391e7f8b45df0b91d3da`. The annotated tag object is `9151b328e76bbc468aa64f85c741886b6227cec1`.
- Fetched: Saturday, October 17, 2026, through the raw-content addresses at that commit, with the User-Agent `diroxide-research (+https://diroxide.app)`.
- Checked: each file's size and git blob hash matched the repository's own listing of the folder at that commit (the GitHub contents API). The README describes every file kept, and every file it describes is here.

## License

The BSD Zero Clause License (0BSD), in `LICENSE`, copied unchanged from the repository's `COPYING.0BSD`. The repository's `COPYING` puts the test files of its `tests` folder under 0BSD, and the README says the same of the test files and of itself. If a notice is wanted, the project gives this one: Copyright (C) The XZ Utils authors and contributors.

## What was kept and left out

- Kept: all 98 files of the folder, 78,829 bytes in all, under the 1 MB allowed for them. That is 71 `.xz` files, 8 `.lzma` files, 18 `.lz` files and the `README` that describes each of them.
- Left out: nothing. Every name is plain (letters, digits, `-`, `_` and `.`), and the folder holds only the test files and their README.
- The two test files that carried the 2024 backdoor of releases 5.6.0 and 5.6.1 (CVE-2024-3094), `bad-3-corrupt_lzma2.xz` and `good-large_compressed.lzma`, are not in this tag's listing and are not here.
- diroxide opens `.xz` files only. The tests run the `.lzma` files through the LZMA decoder that zip and 7z members use, and check that every `.lzma` and `.lz` file is refused in words as a format diroxide does not open.

## Files

Sizes in bytes; the SHA-256 of each file as kept here.

| File | Bytes | SHA-256 |
|---|---|---|
| `README` | 16,612 | 28624c8c7f93a8336ebf929deb839d94f6d575cb102869c474bc5363c322b4ed |
| `bad-0-backward_size.xz` | 32 | 894d4b6d4ea8657893e9ab1c48162438605537e6ff974ee8ee61903b8eaec55a |
| `bad-0-empty-truncated.xz` | 31 | 9de843e125667ecf9b804469889bcea152758585c0e335a7dc15ed243ce83a50 |
| `bad-0-footer_magic.xz` | 32 | b7d60be0dd400c8d8b2a04094d297f4bbf563fd33041cf27ef6e9be74f7df829 |
| `bad-0-header_magic.xz` | 32 | 3adb42fba230b3c09d2277adb7f3ba347f26c1831d4e9514d3068bd3bca9d2a4 |
| `bad-0-index-1.xz` | 32 | 058244ddd11115c6ffa43c3e95820f9ecdae95db4c55a7f6400f7e59e78f952c |
| `bad-0-nonempty_index.xz` | 32 | ece3915eacdc4edc296f593b3cb2617cdd888315eada28dcb366a3fda131590a |
| `bad-0cat-alone.xz` | 55 | 14bbcda4f97f6d3583f36c6a4946d0d0dfb847653175d432ee275d5738d6d9b7 |
| `bad-0cat-header_magic.xz` | 64 | bd3f4dfeae3f4ec3e778809e013859b6b291d18e067c8e9557cc0d4c1009f22e |
| `bad-0catpad-empty.xz` | 69 | 56317222b2ef4743fb18b457c3760094937dacc9fdf18d645c181e5be4b327c6 |
| `bad-0pad-empty.xz` | 37 | a5995e19c63bdb9d327d514c8e8c9d2971b4cfdbc755ffcb11aee30b8a9787b1 |
| `bad-1-block_header-1.xz` | 64 | c8bc15e7bfb1056d358cbe5f9b6ae86489e277e353f275a79a1c212a09cc56af |
| `bad-1-block_header-2.xz` | 64 | 3b0de551b66e8ee65abfaed399e1638de2955aebc882b9585e12d5f93eb21146 |
| `bad-1-block_header-3.xz` | 68 | 2a05eb3e61e10b862f09ddaf030ee5a04ca4c875b2dac251ef0529a8ddbb61f3 |
| `bad-1-block_header-4.xz` | 76 | 8ac58b8fade15875213c116bec05ec78fc659d3cda69116d3aa4dc71557b2d1c |
| `bad-1-block_header-5.xz` | 72 | a76c17d193405f180c3b84c9047ccaf1b4f0483242dda58e7c5070313898f3e9 |
| `bad-1-block_header-6.xz` | 72 | f7cc702ce7a2523e9718816b4a5983b05fa8acbb07341df2a5234ce1828731cb |
| `bad-1-check-crc32-2.xz` | 72 | 7bb7ebde35677c7187ae590a63bfcae24c622a0fc87d479c43f7095efa38f91e |
| `bad-1-check-crc32.xz` | 68 | 4f2de28e30a05d979c7b2db4ba4f4126e92563e3f591bef1508b0926dcf17fa8 |
| `bad-1-check-crc64.xz` | 72 | d7218954fd4bd69eb77b545d8b2428ac35a984ff3bb015846b65d88b325ca219 |
| `bad-1-check-sha256.xz` | 96 | f106a808e57ca48fa4a64b1913042c526c1d777f9eb1622a0857ce18ee34aba3 |
| `bad-1-index-huge-uncomp.xz` | 72 | ad7f4bc132ce5b9ea9d5e34e1deb759027f02163eeab1ad74aa2b9a90afc5d36 |
| `bad-1-lzma2-1.xz` | 64 | b99d620ac64188c4af54e88c79404f153642bed63442a31b642942804f1e1785 |
| `bad-1-lzma2-10.xz` | 60 | 439870b79c45cc862f49a7a13e985b620f617924708d7a2de39826e0b2fff1af |
| `bad-1-lzma2-11.xz` | 64 | 917be0d82e2f4bdc0f00f0f2d9731ba2ea0b8c04c2f2ed21debbc31237d6e926 |
| `bad-1-lzma2-2.xz` | 424 | 91517a1280b5e52ddaa5a327e7c7f5a045d22db94cdc8a2f79781135f461de12 |
| `bad-1-lzma2-3.xz` | 424 | e063697abf2d7cbb7271eaffb064484095abbc2397268cbe11897dca8af52427 |
| `bad-1-lzma2-4.xz` | 408 | 8777af9f9c9e3ddca331d0671a7753a219ef01f3b87708bc5129cc619e63c6e6 |
| `bad-1-lzma2-5.xz` | 408 | 2e2759a097712c49a3e93446276cd56ecb748315c0cdec1d8603d8740c0a1494 |
| `bad-1-lzma2-6.xz` | 68 | 646cb3043b6aac6da1540af308a78e6504c7708af54d5815f224f27a58f00919 |
| `bad-1-lzma2-7.xz` | 408 | ddd7b265fe96595408d72a6664ac3693de097c0d887a9419c5aff3a7fee83d5e |
| `bad-1-lzma2-8.xz` | 464 | 85c49abc062cddf535f41347edb368f702c442241ede7276c4d99b0051f19b0e |
| `bad-1-lzma2-9.xz` | 72 | b193319df9c497d1bfa91c3e008cc4dcd27a4a5aef2ba2c27af73ce52381b578 |
| `bad-1-stream_flags-1.xz` | 68 | c65babcb94c3c175f2d8686391d2d0113fbc434fc8dca009e3f5d1ddf7c83d61 |
| `bad-1-stream_flags-2.xz` | 68 | 35ece04169f64180ba2b4fff03f80a3048ba11b3ef4e16a0ce8d9b5c32ce5e9c |
| `bad-1-stream_flags-3.xz` | 68 | c59b030817cec49b51d1f1b8e9f06ff2535de25e2c4c07a3f45c197d0e3db949 |
| `bad-1-v0-uncomp-size.lz` | 42 | 126278bb766c9b593371693aa7d4b387f3f0832b31a7ac56adaa52a022612881 |
| `bad-1-v1-crc32.lz` | 50 | 99b03c607138206018cc7829051fe0080f4fec421f07f4815c47a65b1a1e35a0 |
| `bad-1-v1-dict-1.lz` | 50 | f37e0fe9cf06c0ac06a8d2fbbb32cecc47855a64c81b723bea43461d17b225f9 |
| `bad-1-v1-dict-2.lz` | 50 | 7578f64931e7178760d679a403cad9d88de0a2521fbc6773ea7004440ea1bf8f |
| `bad-1-v1-magic-1.lz` | 50 | 0e0f70c4f0431d9f4291cc171bebe9a34a2090b0d7375f94306a15ce971a91f1 |
| `bad-1-v1-magic-2.lz` | 50 | 20567dd633fee33a7ebc29bd864ca1304c9c856da6cc58c59656c113e1a03dc8 |
| `bad-1-v1-member-size.lz` | 50 | 175578809d17e055f54b659638963592c32287a8077f0d5a6c74b5fe8ec8446f |
| `bad-1-v1-trailing-magic.lz` | 54 | 61ffe6158813f6a1ef398164e38aa35a98b3321389989f7ecb928f6e1a8d329f |
| `bad-1-v1-uncomp-size.lz` | 50 | 28de1b059fb43dfe9ccb37ad33c01d66c8bf5a5e375011ac41b722314539d380 |
| `bad-1-vli-1.xz` | 72 | dde033a0281c2326178e23ace66268dddac8f5d7d2e585ce865c6f3b4e7ed7da |
| `bad-1-vli-2.xz` | 76 | 9eddf417be15f3f7170fdcdadd753c0fe35d22d13377f271319f0c16889b0e4d |
| `bad-2-compressed_data_padding.xz` | 92 | 9cfe1c1e950111e4563d773790073637c3f76d42d586f10e77469844a0c84dd2 |
| `bad-2-index-1.xz` | 92 | ff6b77b8ce16cdb0b6d455c8045029fa6baec1877798a7e7ed7431b9cec83bca |
| `bad-2-index-2.xz` | 92 | fa394dea8bedb64ad388a8d44ab324cc0aefe9d5b6d7a78b4dfd610ef78263d0 |
| `bad-2-index-3.xz` | 92 | 62f82cabb391bb98ba87162ad04376e2d839741b56eff0750e93cea822b767c8 |
| `bad-2-index-4.xz` | 92 | 102877780afb8155832afd630e426d2e1456f4da62360e4091bec3524d599cec |
| `bad-2-index-5.xz` | 92 | 72ac8e7907c092126bfcc999474e44aa17d29feeeabacc50819a0b5a737afdb3 |
| `bad-3-index-uncomp-overflow.xz` | 132 | a8024da54ce96e32497457b8725d5b04558fe2bd31fa2c47e545b8a347278b34 |
| `bad-too_big_size-with_eopm.lzma` | 37 | 0bbaed2fb52ead1401fe6eb93707e4638d39fe19c58f4931e12b3674a3db06de |
| `bad-too_small_size-without_eopm-1.lzma` | 31 | 3f02afe5a8a334738af731dfb07fc096803a2b3822529a007b0b6a1b950294a7 |
| `bad-too_small_size-without_eopm-2.lzma` | 31 | ddc890ace63cbeb9dc0c377fa058ef282b1d32845e57906554d5b238da731bc8 |
| `bad-too_small_size-without_eopm-3.lzma` | 36 | c73d1c45bac3df79ed5bbe902465d0bb740b0737e57decf99c8975404a190dbd |
| `bad-unknown_size-without_eopm.lzma` | 31 | 27ea24b6918c64d063d796a6759b9d2eae61146e2050e6bd05b4241ae890c6ed |
| `good-0-empty.xz` | 32 | 14c80c40f5b247deead9e9d1d31b8b4f5f0b4f4425e6eb12291fd3e505fc8414 |
| `good-0cat-empty.xz` | 64 | 65de7e01eff8cf2f9a287153971467b1c11811213b64a38bdf673bad240436ff |
| `good-0catpad-empty.xz` | 68 | 019ad3e542d5f5797c6d98148f72725930a057fec6abd9a426ce889302a9fbfe |
| `good-0pad-empty.xz` | 36 | 6b97fc3c4f6f4a9115377b3dce52caf4e726458ed9dfd68b60f94ccdff3c5f8c |
| `good-1-3delta-lzma2.xz` | 528 | 00058b3c84d5e2718c298dd8689730ae79c46995943d242897537e9da3d1b04f |
| `good-1-arm64-lzma2-1.xz` | 512 | 3eb890ddccd424c05a42f897cb0830c8c766ebd60fb2631dd8f9a12c5b374364 |
| `good-1-arm64-lzma2-2.xz` | 488 | a8c7b9fa6e170d1c7a01077bcf197b9b1c69960d99367f10b275f1409e02659c |
| `good-1-block_header-1.xz` | 72 | b94444ef9b5918e4b6c9bcd83080884b61c0c5072588f66c03f244faec475f12 |
| `good-1-block_header-2.xz` | 68 | 96d4c234920acdd9b61ca904b2c38a402706131a5136ec04d98592f755e52664 |
| `good-1-block_header-3.xz` | 68 | 2bb7072476150fdf06cbe83465f443f7928ee011538296f5713e22856b808fb8 |
| `good-1-check-crc32.xz` | 68 | eaa0f7d82fb273920c5247fa4928e695830ce40631945c6dda3cdf3f14d65cec |
| `good-1-check-crc64.xz` | 72 | a0b759464fa8d02f0a3da4d19cebdd1d7d86b19728b599f7f946f58e687a22c1 |
| `good-1-check-none.xz` | 64 | cf3bece11d937fceb9c7599bfeffb067d140441d806823ace927a24e4410c16d |
| `good-1-check-sha256.xz` | 96 | dda8bb24f50e39e750a5ad3697960083450a21ae054b2dff30699d542894826c |
| `good-1-delta-lzma2.tiff.xz` | 51,316 | c9005e580f3e74b2cb8d817120f68cc2177840cda75f48d679551a7f3e323413 |
| `good-1-empty-bcj-lzma2.xz` | 52 | 2a05dacdba9c472f4872416211c80bad48e6ee9e4bb6d7f4b8f589009376166f |
| `good-1-lzma2-1.xz` | 424 | d0e6089fe87776cb754b08012cc05b922e2d04f8a5484baf2d946cceebc24546 |
| `good-1-lzma2-2.xz` | 424 | 8d8d4708327aa13930373d80ce3b996ccf0f1ff67d3cdd3d494aa6360fa53537 |
| `good-1-lzma2-3.xz` | 408 | 2c286d6b18f24301f25a1c501a2c74c2036c94e0ab85d2289991f7fca2acb3ad |
| `good-1-lzma2-4.xz` | 464 | ee91a262eaac56d93ffe9b3d582bcfdc1d92a8edb3a5555d52adc4c21f5347f1 |
| `good-1-lzma2-5.xz` | 52 | a59afee16f97700f89a3cc81740058a0e64fc9e06d176450b44aff3d70716c4f |
| `good-1-v0-trailing-1.lz` | 59 | 7441d2ee40b5581cc8880321713532c68e5d4478a3cbcb8539c894b248f79cff |
| `good-1-v0.lz` | 42 | c1d31ff8e54277f124f6c3540372be95ca44256357b9ce27bba166794356b82c |
| `good-1-v1-trailing-1.lz` | 67 | 7b1ca35213de9c8f674b786c44e96e0200b6245284324fd3317d96a52d12cb30 |
| `good-1-v1-trailing-2.lz` | 70 | d6d410dfc509e0d93f234897f9a70a7854248e74e40c97ca681a5575602b3021 |
| `good-1-v1.lz` | 50 | 1b06c015c7eba8e3b826422a0c281f1acbeb403703d78048d9dd27b9a54dbafd |
| `good-2-lzma2.xz` | 92 | c096d08c0effa829b9c1cb92242b80e26be3a206f7a503d73b552da205b64d6d |
| `good-2-v0-v1.lz` | 78 | 0511f25c84c46cd2e20158800d7fd2b180606e59bfaea8b4ab9bf62e140f59a7 |
| `good-2-v1-v0.lz` | 78 | ecb5b06ed6c2f5f56f727ec9dce98e9755d5d3e38d2550198f755e702a288b21 |
| `good-2-v1-v1.lz` | 86 | dc3da6b1e79a64f7b079b95b3cf8277570aa324866da702fb0504616d8aabee0 |
| `good-known_size-with_eopm.lzma` | 37 | 392b7e5739d04e59c42de50129b6f7a36193c73399e8a430a1a9a7b1d9a65c18 |
| `good-known_size-without_eopm.lzma` | 31 | fe595cfd83f1d76eb5ded06d66f63e1a91d5e129558eacb1d0fb8ab030bbfeb8 |
| `good-unknown_size-with_eopm.lzma` | 37 | 59e8f4f4dcc099085821d8c5736e6757a84f5d1575ce2948eee130eaf964c035 |
| `unsupported-1-v234.lz` | 50 | 64bdb715f44d3cfef96dbc39e93c77124fa7de0ec3d10756803459f806135cdd |
| `unsupported-block_header.xz` | 68 | acf4e10b6f8e885ca80cf27c36187cafd974af46b42f70a761d969f5c0edbcaf |
| `unsupported-check.xz` | 68 | c781077a53fcb200131fc6aaa3eaed6140f71f8cea5bcf9aae6898c5df4371bd |
| `unsupported-filter_flags-1.xz` | 68 | adb9d23d6645ffa0a4dfd377cb8fb6a01a2622edc1c8bf1ed1ddc91b45b6d5f1 |
| `unsupported-filter_flags-2.xz` | 68 | e7cd859ba5efd8e238fd4bb44724ee3ee27fa2c993cc8794330e97867d137eff |
| `unsupported-filter_flags-3.xz` | 68 | 03b202ddc097d5918b808c2de99b37fbf1d138274998911bff2d514d1a04f109 |
