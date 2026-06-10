# 贡献指南

感谢您对 BashRS 项目的兴趣！我们欢迎各种形式的贡献，包括但不限于代码、文档、测试和反馈。

## 行为准则

请阅读我们的 [Code of Conduct](CODE_OF_CONDUCT.md)，并确保在所有社区互动中遵守。

## 如何贡献

### 报告问题

如果您发现了 bug 或有功能请求，请通过 GitHub Issues 提交：

1. 搜索现有问题，避免重复
2. 使用清晰的问题模板
3. 提供复现步骤和预期行为
4. 包含您的环境和版本信息

### 提交代码

#### 1. Fork 并克隆仓库

```bash
git clone https://github.com/YOUR_USERNAME/bashrs.git
cd bashrs
```

#### 2. 创建功能分支

```bash
git checkout -b feature/your-feature-name
# 或
git checkout -b fix/bug-description
```

#### 3. 开发 (使用 TDD 方法)

我们使用测试驱动开发 (TDD) 方法：

```bash
# 1. 先编写测试
# 编辑 tests/ 目录下的测试文件

# 2. 运行测试确认失败
cargo test --test your_test_file

# 3. 实现功能
# 编辑 src/ 目录下的源代码

# 4. 确认测试通过
cargo test

# 5. 重构 (可选但推荐)
# 简化代码，确保测试仍然通过
cargo test
```

#### 4. 提交更改

```bash
git add .
git commit -m "描述您的更改"
```

提交信息格式：

```
<类型>: <简短描述>

<详细描述 (可选)>

<相关 issue (可选)>
```

类型可以是：
- `feat`: 新功能
- `fix`: 错误修复
- `docs`: 文档更新
- `test`: 测试相关
- `refactor`: 重构
- `perf`: 性能优化

#### 5. 推送并创建 PR

```bash
git push origin feature/your-feature-name
```

然后在 GitHub 上创建 Pull Request。

### 代码规范

#### Rust 代码风格

- 使用 `rustfmt` 格式化代码
- 遵循 Rust 命名约定
- 添加必要的文档注释
- 避免不必要的 `unwrap()`

```bash
# 格式化代码
cargo fmt

# 检查代码
cargo clippy
```

#### 测试要求

- 所有新功能必须有测试
- 修复 bug 时添加回归测试
- 测试应该清晰、可读
- 遵循现有的测试结构

```rust
// 测试模块示例
#[cfg(test)]
mod your_tests {
    use super::*;

    #[test]
    fn test_your_feature() {
        // 测试实现
    }
}
```

#### 命名规范

| 类型 | 规范 | 示例 |
|------|------|------|
| 模块 | 小写下划线 | `lexer`, `parser` |
| 函数 | 小写下划线 | `tokenize()`, `parse()` |
| 结构体 | 大驼峰 | `Lexer`, `Token` |
| 枚举 | 大驼峰 | `TokenKind` |
| 常量 | 全大写下划线 | `MAX_SIZE` |
| 测试函数 | 小写下划线 | `test_basic_token` |

### 提交信息规范

好的提交信息示例：

```
feat: 添加变量展开支持

- 实现 $VAR 和 ${VAR} 语法
- 添加环境变量查询
- 更新测试覆盖率

Closes #123
```

## 开发工作流

### 分支策略

- `main`: 稳定代码
- `feature/*`: 新功能开发
- `fix/*`: bug 修复
- `refactor/*`: 重构

### 代码审查

所有 PR 需要：
- 通过所有测试
- 通过 CI 检查
- 至少一个维护者批准
- 无冲突

### 测试覆盖率

我们的目标是最小 80% 的测试覆盖率：

```bash
# 查看测试覆盖率
cargo test -- --nocapture
# 未来会添加 coverage 工具
```

## 常见问题

### Q: 如何运行特定测试？
```bash
cargo test test_function_name
```

### Q: 如何调试？
```bash
RUST_BACKTRACE=1 cargo test
```

### Q: 代码风格有问题？
```bash
cargo fmt
cargo clippy --fix
```

## 资源

- [Rust 官方文档](https://doc.rust-lang.org/)
- [Rust 标准库](https://doc.rust-lang.org/std/)
- [Rust 测试框架](https://doc.rust-lang.org/book/ch11-00-testing.html)

## 联系方式

如有疑问，请：
- 创建 GitHub Discussion
- 发送邮件到 maintainers

## 感谢

感谢所有贡献者的努力！