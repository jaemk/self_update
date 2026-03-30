
CHECK="${1:-}"
if [[ "$CHECK" == "check" ]]; then
    echo "checking README.md..."
	cargo readme --no-indent-headings > _tmp_readme.md
	cmp README.md _tmp_readme.md
    rc=$?
	rm -f _tmp_readme.md
    exit $rc
else
    echo "generating README.md..."
    cargo readme --no-indent-headings > README.md
fi
