# The grouped script driver (scripts mentioning expand_aliases) must keep a
# command group open across a physical line break after a binary connector:
# GNU parse.y consumes the newline as part of the production
# (`list1: list1 AND_AND newline_list list1` parse.y:1286, `pipeline '|'
# newline_list pipeline` parse.y:1471). rubash#255: the group closed after
# the `&&` line and the truncated parse died with `syntax error: unexpected
# end of file`.
# expand_aliases
[[ ${v-} != reload ]] &&
  v=set
echo "v=${v-unset}"
echo a &&
  echo b
echo p |
  echo q
if true; then
  echo c &&
    echo d
fi
echo x &&
echo y
echo done
