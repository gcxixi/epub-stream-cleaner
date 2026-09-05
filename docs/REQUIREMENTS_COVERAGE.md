# 需求覆盖矩阵

本文对应会议评审提出的“四大防御铁律”和目录重构要求。矩阵以当前 `main` 为准；发布标签只有在对应提交之后才代表同等能力。

| 需求 | 当前状态 | 现状与验收证据 |
|---|---|---|
| 外部协议链接与内部跳转区分 | ✅ 已覆盖 | 任意带 URI scheme 的 `a[href]` 默认移除并保留文本；相对路径、`#fragment`、跨 XHTML 引用保留；`epub:type` 含 `noteref` 或 `role` 含 `doc-noteref` 时硬保护。 |
| XHTML/XML 良好成形与命名空间 | ✅ 已覆盖（失败即不提交） | `.xhtml` 改写前后均做 XML 解析校验，并比较根节点默认命名空间；验证失败时临时输出不会替换目标文件。真实 EPUB fixture 覆盖 `xmlns` 与 `<br/>`。 |
| 广告图片结构 + 图片特征双重判定 | 🟨 保守子集 | 当前只按容器 `id/class` 的完整 token 命中并 unwrap，保留所有后代图片；尚未按真实图片二进制计算 pHash/尺寸比例，也不会删除图片文件。 |
| 删除图片时同步 OPF manifest | 🟨 不触发删除路径 | 当前策略不删除图片，因此不会制造悬空 manifest 引用；真正的图片删除与 OPF manifest/spine 一致性校验仍未实现。 |
| CSS 增量覆盖 | ✅ 已覆盖 | 不读取、不重写 CSS 文件，不改变已有 class 名；仅对明确命中的 XHTML 元素做流式增量改写。 |
| OCF 容器约束 | ✅ 基础约束已覆盖 | `mimetype` 必须首条、Stored、内容精确；校验路径安全、重复条目、`container.xml`、XML/XHTML，并在原子提交前复验输出。 |
| EPUBCheck 全规范与 manifest/spine 引用一致性 | 🟨 未完全覆盖 | 当前是 OCF 关键预检，不等价于完整 EPUBCheck；尚未建立 manifest/spine/资源引用的全量一致性检查。 |
| 保留已有多级目录 | ✅ 当前不会改写 | OPF、NCX、nav.xhtml 当前只做 XML 校验，不主动改写已有目录。 |
| 缺失/扁平目录自动 AST 重构 | ❌ 未实现 | 当前没有遍历 XHTML 标题、中文章节正则、nav.xhtml/NCX 生成与 OPF 挂载逻辑。 |

## 当前 Goal 验收

- CI：`fmt / clippy / test`、Ubuntu、macOS、Windows 全部通过。
- 集成测试：真实 ZIP/EPUB fixture 覆盖严格 XHTML、语义脚注保护、外链剥离与幂等性。
- 当前可发布能力：保守清洗 MVP。
- 尚不能宣称：广告图片 pHash/比例双重删除、删除后的 manifest 自动注销、缺失目录自动重构、完整 EPUBCheck 等价验证。

## 发布门槛

在补齐 🟨/❌ 项目之前，发布说明必须把它们作为明确边界；补齐后需要新增 golden fixture、manifest 引用一致性测试、目录层级测试，并在真实样本集上执行 EPUBCheck，再创建新的版本标签。
