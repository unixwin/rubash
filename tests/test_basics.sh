#!/bin/bash
# 基本语法测试

echo "=== 基本语法测试 ==="

# 1. 变量赋值
name="World"
echo "1. 变量赋值: Hello, $name!"

# 2. 命令替换
current_dir=$(pwd)
echo "2. 命令替换: $current_dir"

# 3. 算术运算
a=10
b=3
echo "3. 算术运算: $a + $b = $((a + b))"
echo "   乘法: $a * $b = $((a * b))"
echo "   除法: $a / $b = $((a / b))"
echo "   取余: $a % $b = $((a % b))"

# 4. 字符串操作
str="Hello World"
echo "4. 字符串长度: ${#str}"
echo "   子字符串: ${str:0:5}"
echo "   替换: ${str/World/Bash}"

# 5. 引号
echo "5. 双引号: \"$name\""
echo '   单引号: "$name"'

# 6. 转义字符
echo "6. 制表符:\tTab"
echo "   换行符:\nNewline"

# 7. 特殊变量
echo "7. 脚本名: $0"
echo "   参数个数: $#"
echo "   所有参数: $@"
echo "   进程ID: $$"
echo "   上一命令状态: $?"

echo "=== 基本语法测试完成 ==="
