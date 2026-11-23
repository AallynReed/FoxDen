# Release notes for version $v from changelog.json.
[["added", "New"], ["devices", "Device support"], ["changed", "Changes"], ["fixed", "Fixes"], ["removed", "Removed"]] as $sections
| (.[] | select(.version == $v)) as $e
| [$sections[] | select(($e[.[0]] // []) | length > 0)
   | "### \(.[1])\n" + ([$e[.[0]][] | "- \(.)"] | join("\n"))]
| join("\n\n")
