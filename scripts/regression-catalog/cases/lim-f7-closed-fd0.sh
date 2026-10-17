echo before
exec 0<&-
read y < /dev/null
echo "rc=$? y=[$y]"
echo after
