#!/bin/bash
# 运行所有测试脚本

echo "=========================================="
echo "  bashrs 功能测试套件"
echo "=========================================="
echo ""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TOTAL=0
PASSED=0
FAILED=0

# 查找 rubash 可执行文件
find_rubash() {
    # 1. 检查环境变量
    if [ -n "$RUBASH_PATH" ] && [ -x "$RUBASH_PATH" ]; then
        echo "$RUBASH_PATH"
        return 0
    fi

    # 2. 检查 cargo build 输出
    local debug_path="$PROJECT_DIR/target/debug/rubash"
    local debug_exe_path="$PROJECT_DIR/target/debug/rubash.exe"
    local release_path="$PROJECT_DIR/target/release/rubash"
    local release_exe_path="$PROJECT_DIR/target/release/rubash.exe"

    if [ -x "$debug_path" ]; then
        echo "$debug_path"
        return 0
    elif [ -x "$debug_exe_path" ]; then
        echo "$debug_exe_path"
        return 0
    elif [ -x "$release_path" ]; then
        echo "$release_path"
        return 0
    elif [ -x "$release_exe_path" ]; then
        echo "$release_exe_path"
        return 0
    fi

    # 3. 检查 PATH 中的 rubash
    if command -v rubash >/dev/null 2>&1; then
        echo "rubash"
        return 0
    fi

    # 4. 回退到 bash
    echo "bash"
    return 1
}

RUBASH=$(find_rubash)
if [ "$RUBASH" = "bash" ]; then
    echo "警告: 未找到 rubash，使用系统 bash 运行测试"
else
    echo "使用 rubash: $RUBASH"
fi
echo ""

run_test() {
    local test_file="$1"
    local test_name="$(basename "$test_file" .sh)"

    echo "运行测试: $test_name"
    echo "------------------------------------------"

    if $RUBASH "$test_file" 2>&1; then
        echo "✓ $test_name 通过"
        PASSED=$((PASSED + 1))
    else
        echo "✗ $test_name 失败"
        FAILED=$((FAILED + 1))
    fi

    TOTAL=$((TOTAL + 1))
    echo ""
}

# 运行所有测试
for test_file in "$SCRIPT_DIR"/test_*.sh; do
    if [ -f "$test_file" ]; then
        run_test "$test_file"
    fi
done

echo "=========================================="
echo "  测试结果汇总"
echo "=========================================="
echo "总计: $TOTAL"
echo "通过: $PASSED"
echo "失败: $FAILED"
echo ""

if [ $FAILED -eq 0 ]; then
    echo "✓ 所有测试通过！"
    exit 0
else
    echo "✗ 有 $FAILED 个测试失败"
    exit 1
fi
