#!/bin/bash
# 数组测试

echo "=== 数组测试 ==="

# 1. 索引数组
echo "1. 索引数组:"
arr=("apple" "banana" "cherry")
echo "   所有元素: ${arr[@]}"
echo "   第一个元素: ${arr[0]}"
echo "   第二个元素: ${arr[1]}"
echo "   数组长度: ${#arr[@]}"

# 2. 数组追加
echo "2. 数组追加:"
arr+=("date")
arr+=("elderberry")
echo "   追加后: ${arr[@]}"

# 3. 数组切片
echo "3. 数组切片:"
echo "   切片 [1:3]: ${arr[@]:1:3}"

# 4. 关联数组
echo "4. 关联数组:"
declare -A colors
colors[red]="#ff0000"
colors[green]="#00ff00"
colors[blue]="#0000ff"
echo "   红色: ${colors[red]}"
echo "   所有键: ${!colors[@]}"
echo "   所有值: ${colors[@]}"

# 5. 数组遍历
echo "5. 数组遍历:"
for item in "${arr[@]}"; do
    echo "   - $item"
done

# 6. 数组索引遍历
echo "6. 数组索引遍历:"
for i in "${!arr[@]}"; do
    echo "   [$i] = ${arr[$i]}"
done

# 7. 数组删除
echo "7. 数组删除:"
unset arr[1]
echo "   删除索引1后: ${arr[@]}"

# 8. 数组赋值
echo "8. 数组赋值:"
new_arr=($(echo -e "one\ntwo\nthree"))
echo "   新数组: ${new_arr[@]}"

# 9. 数组长度
echo "9. 数组长度:"
echo "   数组长度: ${#new_arr[@]}"
echo "   第一个元素长度: ${#new_arr[0]}"

# 10. 数组作为参数
echo "10. 数组作为参数:"
print_array() {
    local -a arr=("$@")
    echo "    数组内容: ${arr[@]}"
    echo "    数组长度: ${#arr[@]}"
}
print_array "${arr[@]}"

# 11. 数组复制
echo "11. 数组复制:"
original=("a" "b" "c")
copy=("${original[@]}")
echo "    原数组: ${original[@]}"
echo "    复制数组: ${copy[@]}"

# 12. 数组合并
echo "12. 数组合并:"
arr1=("one" "two")
arr2=("three" "four")
merged=("${arr1[@]}" "${arr2[@]}")
echo "    合并后: ${merged[@]}"

echo "=== 数组测试完成 ==="
