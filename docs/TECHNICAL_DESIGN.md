# 基于 Rust 的大规模 EPUB 高保真流式清洗方案

## 1. 目标与非目标

### 目标

本项目处理的是“已有 EPUB 的保守清洗”，重点是：

- 大文件不构建完整 DOM，不把整个 EPUB 或整个 XHTML 读入内存；
- 外部垃圾锚点可去除，同时保护内部章节跳转、脚注、尾注和目录锚点；
- 广告容器命中规则可解释，插图元素不被误删；
- 输出始终满足 OCF 关键约束；
- 单文件失败不污染旧输出，批处理可以利用多核；
- 每次运行产生可机器消费的 JSON 统计。

### 非目标

- 不做任意 CSS 重排、字体替换、图片重编码或语义摘要；
- 不承诺第三方 EPUB 的所有扩展 XML 都能被 HTML 重写器接受；
- 不将“删除所有网络请求”作为默认策略，因为远程字体、封面或媒体可能是合法内容；
- 不在运行时依赖 Calibre、Python 或浏览器。

## 2. 架构

~~~mermaid
flowchart TD
    A[EPUB ZIP] --> B[OCF 预检]
    B --> C{条目类型}
    C -->|XHTML/HTML| D[lol-html 分块改写]
    C -->|XML/OPF/NCX| E[仅 XML 校验]
    C -->|图片/字体/媒体| F[顺序复制]
    D --> G[条目临时文件]
    E --> G
    F --> H[ZIP Writer]
    G --> H
    H --> I[输出 OCF 校验]
    I --> J[原子替换]
~~~

### 单 EPUB 数据流

1. ZipArchive 顺序读取中央目录。
2. 首条必须是未压缩的 mimetype，且内容必须精确匹配。
3. XHTML/HTML 以 64 KiB 块喂给 lol-html，输出写入当前条目的临时文件。
4. 条目级临时文件完成后做 XML 良好成形校验，再写入输出 ZIP。
5. 全部条目写完后重新打开输出 ZIP，复验 OCF 关键约束。
6. 通过校验才将临时输出原子替换为目标路径。

单个 EPUB 的 ZIP 写出不能并行，因为中央目录和条目顺序需要确定；目录批处理才是并行边界。clean_batch 用 Rayon 对不同 EPUB 独立执行，任一文件失败不会回滚已经成功提交的其他文件。

## 3. 清洗策略

### 3.1 外部链接

只处理带 href 的 a 元素：

- 任意带 URI scheme 的 URL（例如 http://、https://、ftp://、mailto:）以及协议相对 URL //host/path 视为外部链接；
- #fragment、chapter.xhtml#fragment、../Text/chapter.xhtml 保留；
- `epub:type` 含 `noteref` 或 `role` 含 `doc-noteref` 的语义脚注硬保护，即使 href 带外部 scheme 也不 unwrap；
- 命中后使用“移除元素、保留内容”的操作，因此正文文字不丢失；
- 不自动删除图片、字体、视频或 CSS 的外部 URL。

这样可以保护脚注、尾注、目录跳转、章节内部引用和跨文件引用。

### 3.2 广告容器

默认只在常见容器元素上检查 id 与 class：

div, section, article, aside, header, footer, p, table, ul, ol

标记按空白、-、_、:、. 切分，只匹配完整 token，例如：

- 会命中：ad-banner、sponsored、advertisement；
- 不会命中：address-card、chapter。

图片节点本身不在可删除选择器中；命中的广告容器默认只移除容器包装、保留其后代内容，因此广告容器里的插图也能安全保留。如果业务需要删除容器内图片，需要单独增加经过样本验证的策略，而不是扩大默认黑名单。

当前版本没有执行“外链 + 图片长宽比/pHash”的图片删除路径，也没有从 OPF manifest 注销图片。这样做是有意的保守降级：不会出现图片文件已删、manifest 仍引用的 EPUB 破坏问题；该能力属于后续版本，而不是当前已覆盖能力。

### 3.3 高保真边界

`.xhtml` 条目在改写前先做 XML 良好成形校验，改写后再次校验，并比较根节点默认 namespace。任何校验失败都会阻止原子替换。当前 CI 集成 fixture 覆盖 XHTML namespace、XML 自闭合标签、普通外链和语义脚注。

### 3.4 高保真边界

lol-html 会对被它触及的 HTML 标签进行重新序列化，因此“字节级原样直通”应限定为：

- 输入不命中任何策略的内容在语义上直通；
- 图片、字体、媒体、OPF 等非目标资源不被 HTML 清洗器解析；
- ZIP 层面会发生重新压缩，不能承诺压缩后二进制完全相同。

需要字节级审计时，建议保存输入/输出的每个条目哈希，并对允许改变的 XHTML 条目执行结构化 diff。当前 JSON report 记录条目数量、字节数和命中计数，后续可以增加条目 SHA-256。

## 4. OCF 与安全校验

当前实现执行以下校验：

- ZIP 非空；
- mimetype 是第一条目；
- mimetype 使用 Stored/零压缩；
- mimetype 内容严格为 application/epub+zip；
- ZIP 条目名称不允许绝对路径或 .. 路径片段；
- 不允许重复条目名；
- 单条目解压大小默认不超过 256 MiB；
- META-INF/container.xml 必须存在且 XML 良好成形；
- 处理后的 .xml、.opf、.ncx、.xhtml 做 XML 解析校验；
- 清洗输出在原子提交前再次执行 OCF 预检。

下一阶段建议增加：压缩比阈值、总解压大小阈值、加密 EPUB 明确拒绝、所有清洗条目 SHA-256、XML 外部实体策略和 manifest/spine 引用一致性检查。

## 5. 性能模型

### 内存

单个 XHTML 清洗器的常驻工作内存约为：

O(rewriter state + chunk size + current entry spool buffers)

默认输入块是 64 KiB，临时文件只承载当前条目，不会把整本书载入内存。ZipArchive 本身仍需要读取中央目录，这是 ZIP 格式的正常成本。

### 吞吐

影响吞吐的主要因素：

- XHTML 比例与文本结构复杂度；
- 输出压缩级别；
- 存储设备的顺序写能力；
- 批处理中的 EPUB 数量与 Rayon 线程数。

建议基准：

- 1 MiB、50 MiB、500 MiB、2 GiB EPUB 分层；
- 纯文本、图片密集、字体密集三类；
- 1/2/4/8/16 并发；
- 记录 p50/p95 延迟、MiB/s、峰值 RSS、命中率和输出体积变化。

## 6. 工程化落地

### 第一阶段：可用 MVP

- 当前仓库的单文件 clean；
- 目录级 batch；
- 默认保守规则；
- OCF 校验；
- JSON 报告；
- CI 和多平台 Release。

### 第二阶段：审计与规则配置

- TOML/JSON 规则文件；
- dry-run 模式；
- 条目 SHA-256 和结构化变更摘要；
- 规则命中上下文采样；
- allowlist/denylist 域名；
- 自定义广告类名词典。

### 第三阶段：规模化分发

- 服务化 API 或消息队列消费者；
- 对象存储输入/输出；
- 内容寻址缓存；
- 并发限额和租户隔离；
- Prometheus 指标；
- 失败样本自动归档；
- 规则版本与产物版本绑定。

## 7. 目录策略

当前版本对 `content.opf`、`toc.ncx`、`nav.xhtml` 只做 XML 校验，不自动重构目录。已有目录不会被改写；缺失或扁平目录的 AST 提取、中文章节正则、nav.xhtml/NCX 生成与 OPF 挂载尚未实现，不能在当前版本宣称覆盖该议程项。

## 8. 异常用例矩阵

| 用例 | 预期 |
|---|---|
| mimetype 不在第一位 | 拒绝输入 |
| mimetype 被压缩 | 拒绝输入 |
| container.xml 缺失 | 拒绝输入 |
| XML 标签未闭合 | 拒绝输入或不提交输出 |
| 内部脚注 #note-1 | 保留 |
| 跨文件引用 Text/ch2.xhtml#p1 | 保留 |
| 外部 ftp:// / https:// 锚点 | 默认保留正文、移除锚点 |
| epub:type="noteref" 外部 href | 保留完整语义链接 |
| address-card 类名 | 不当作广告 |
| ad-banner 容器 | 默认移除 |
| 广告容器内 figure/img | 移除广告容器包装，保留 figure/img 内容 |
| ZIP ../evil 条目 | 拒绝输入 |
| 超大单条目 | 按上限拒绝 |
| 输出路径已存在 | 仅在全流程成功并复验后替换 |

## 9. 验收标准

1. 真实样本集在启用默认规则后，未命中条目的结构化内容无变化。
2. 100 MiB 以上 XHTML 不因全量 DOM 构建导致内存线性膨胀。
3. 所有输出通过 EPUBCheck；本项目自身至少保证 OCF 关键预检与 XML 良好成形。
4. CI 在 Linux、macOS、Windows 上通过格式、测试、Clippy。
5. 打标签后 Release 自动提供五个平台二进制和 SHA-256。
