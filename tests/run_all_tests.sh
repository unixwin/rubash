#!/bin/bash
# 运行所有测试脚本

echo "=========================================="
echo "  bashrs 功能测试套件"
echo "=========================================="
echo ""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TOTAL=0
PASSED=0
FAILED=0

run_test() {
    local test_file="$1"
    local test_name="$(basename "$test_file" .sh)"

    echo "运行测试: $test_name"
    echo "------------------------------------------"

    if bash "$test_file" 2>&1; then
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
