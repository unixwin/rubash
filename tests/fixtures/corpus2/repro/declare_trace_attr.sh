f() { :; }
declare -ft f
declare -F
h() { :; }
declare -f -t h
echo "after_sep_flags=$?"
