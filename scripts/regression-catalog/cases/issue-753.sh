shopt -s failglob
echo "c1-start"
( set -- /no/such/*.zzz; echo "c1-inside" ); echo "c1-rc=$?"
echo "c2-start"
{ set -- /no/such/*.zzz; echo "c2-inside"; }; echo "c2-rc=$?"
echo "c3-start"
set -- /no/such/*.zzz; echo "c3-rc=$?"
echo "END"
