#!/bin/bash
# 控制流测试

echo "=== 控制流测试 ==="

# 1. if 语句
echo "1. if 语句:"
x=10
if [ $x -gt 5 ]; then
    echo "   $x > 5"
elif [ $x -eq 5 ]; then
    echo "   $x == 5"
else
    echo "   $x < 5"
fi

# 2. [[ ]] 条件
echo "2. [[ ]] 条件:"
if [[ "hello" == h* ]]; then
    echo "   模式匹配成功"
fi

if [[ "abc123" =~ ^[a-z]+[0-9]+$ ]]; then
    echo "   正则匹配成功"
fi

# 3. while 循环
echo "3. while 循环:"
counter=0
while [ $counter -lt 3 ]; do
    echo "   计数: $counter"
    counter=$((counter + 1))
done

# 4. until 循环
echo "4. until 循环:"
counter=3
until [ $counter -le 0 ]; do
    echo "   倒计时: $counter"
    counter=$((counter - 1))
done

# 5. for 循环
echo "5. for 循环:"
for i in 1 2 3; do
    echo "   数字: $i"
done

# 6. for 循环（范围）
echo "6. for 循环（范围）:"
for i in {1..5}; do
    echo -n "$i "
done
echo ""

# 7. case 语句
echo "7. case 语甸:"
fruit="apple"
case "$fruit" in
    apple)
        echo "   这是苹果"
        ;;
    banana)
        echo "   这是香蕉"
        ;;
    *)
        echo "   未知水果"
        ;;
esac

# 8. break 和 continue
echo "8. break 和 continue:"
for i in {1..10}; do
    if [ $i -eq 3 ]; then
        continue
    fi
    if [ $i -eq 7 ]; then
        break
    fi
    echo -n "$i "
done
echo ""

# 9. 嵌套循环
echo "9. 嵌套循环:"
for i in 1 2; do
    for j in a b; do
        echo -n "$i$j "
    done
done
echo ""

# 10. select 语句（非交互式测试）
echo "10. select 语句（模拟）:"
options=("选项1" "选项2" "选项3")
for i in "${!options[@]}"; do
    echo "   $((i+1)). ${options[$i]}"
done

echo "=== 控制流测试完成 ==="
