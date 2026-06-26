#!/bin/bash
# 函数测试

echo "=== 函数测试 ==="

# 1. 基本函数
echo "1. 基本函数:"
greet() {
    echo "   Hello, $1!"
}
greet "World"

# 2. 带返回值的函数
echo "2. 带返回值的函数:"
add() {
    local a=$1
    local b=$2
    echo $((a + b))
}
result=$(add 3 4)
echo "   3 + 4 = $result"

# 3. 多个参数
echo "3. 多个参数:"
print_args() {
    echo "   参数个数: $#"
    echo "   所有参数: $@"
    echo "   第一个参数: $1"
    echo "   第二个参数: $2"
}
print_args "hello" "world" "foo"

# 4. 局部变量
echo "4. 局部变量:"
test_local() {
    local x=10
    echo "   函数内: x=$x"
}
test_local
echo "   函数外: x=${x:-未定义}"

# 5. 递归函数
echo "5. 递归函数:"
factorial() {
    local n=$1
    if [ $n -le 1 ]; then
        echo 1
    else
        local prev=$(factorial $((n - 1)))
        echo $((n * prev))
    fi
}
echo "   5! = $(factorial 5)"

# 6. 函数作为参数
echo "6. 函数作为参数:"
apply() {
    local func=$1
    local arg=$2
    $func "$arg"
}
to_upper() {
    echo "${1^^}"
}
apply to_upper "hello"

# 7. 匿名函数（子shell）
echo "7. 匿名函数（子shell）:"
(
    x="子shell变量"
    echo "   $x"
)

# 8. 函数导出
echo "8. 函数导出:"
my_func() {
    echo "   导出的函数"
}
export -f my_func
# 注意：导出的函数在子shell中可用

# 9. 陷阱函数
echo "9. 陷阱函数:"
cleanup() {
    echo "   清理函数被调用"
}
trap cleanup EXIT

# 10. 函数覆盖
echo "10. 函数覆盖:"
original() {
    echo "   原始函数"
}
original() {
    echo "   覆盖后的函数"
}
original

echo "=== 函数测试完成 ==="
