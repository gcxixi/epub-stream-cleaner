# epub-stream-cleaner

一个面向大规模 EPUB 批处理的 Rust 流式清洗器。它使用 lol-html 对 XHTML 做增量改写，使用 ZIP 顺序读写保持 OCF 容器约束，并使用 Rayon 并行处理相互独立的 EPUB 文件。

当前版本的默认策略是：

- 移除外部 HTTP(S) 锚点，但保留锚点文本；
- 保留 #footnote-1、chapter.xhtml#p3 等内部语义链接；
- 按 id/class 中的精确 token 过滤常见广告容器；
- 不删除 img、figure、SVG 等插图节点；
- 强制 mimetype 为第一个条目、零压缩、内容严格等于 application/epub+zip；
- 对 container.xml 与处理过的 XML/XHTML 做良好成形校验；
- 采用临时文件与原子替换，失败时不破坏已有输出。

> “零误伤”应理解为可审计的保守策略，而不是对所有未知 EPUB 内容作绝对承诺。默认只改写明确命中的锚点和容器，建议上线前用真实样本集做 golden diff。

## 快速开始

需要 Rust 1.80 或更高版本：

~~~bash
cargo install --path .
epub-clean clean book.epub book.cleaned.epub --report report.json
~~~

批处理：

~~~bash
epub-clean batch ./incoming ./cleaned --report batch-report.json
~~~

保守开关：

~~~bash
epub-clean clean in.epub out.epub \
  --keep-external-links \
  --keep-ad-containers \
  --max-entry-mib 512
~~~

## 工程结构

~~~text
src/lib.rs                         清洗、OCF 校验、原子输出、批处理 API
src/main.rs                        clean/batch CLI
docs/TECHNICAL_DESIGN.md           评审级技术方案与落地计划
tests/                             集成测试放置目录
.github/workflows/ci.yml           格式、测试、Clippy
.github/workflows/release.yml      多平台构建与 GitHub Release
~~~

## Release

推送版本标签即可触发发布：

~~~bash
git tag v0.1.0
git push origin v0.1.0
~~~

Workflow 会构建 Linux x86_64/ARM64、macOS x86_64/ARM64 和 Windows x86_64，并上传压缩包与 SHA-256 校验文件。

## 已知边界

1. 单个 EPUB 内部的 ZIP 写出必须是顺序的；真正的并行粒度是多个 EPUB 文件。批处理使用 Rayon。
2. lol-html 是流式 HTML/XHTML 重写器，不是任意 XML 的通用编辑器。OPF、NCX、container.xml 只做校验，不做 HTML 规则改写。
3. EPUB 内部的远程图片、字体和媒体不会因为是外部 URL 而自动删除，避免破坏合法资源。
4. 输入压缩包中的普通资源会解压后重新压缩，因此不能承诺 ZIP 层面的字节级不变；未命中的内容在语义上保持不变。
5. 默认单条目解压上限为 256 MiB，用于降低 ZIP bomb 风险，可通过参数调整。

## License

MIT
