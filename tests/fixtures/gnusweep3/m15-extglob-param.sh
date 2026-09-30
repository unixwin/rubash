shopt -s extglob
v=abcabc
echo "${v/@(abc)/X}"
echo "${v//@(abc)/X}"
echo "${v/!(a)/Z}"
