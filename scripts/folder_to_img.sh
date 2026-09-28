set -euo pipefail

if [ "$#" -ne 5 ]; then
    echo "usage: $0 <image> <size> <label> <serial> <source_dir>" >&2
    exit 1
fi

IMAGE="$1"
SIZE="$2"
LABEL="$3"
SERIAL="$4"
SOURCE_DIR="$5"

# 2026-01-01T00:00:00Z
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1767225600}"
export SOURCE_DATE_EPOCH

# Create a FAT16 image from the specified folder, deleting any existing image first.
rm -f "$IMAGE"
truncate -s "$SIZE" "$IMAGE"
mkfs.fat -F 16 -i "$SERIAL" "$IMAGE"
mlabel -i "$IMAGE" "::$LABEL"

# Copy directories into the FAT16 image.
while IFS= read -r -d '' dir; do
    rel="${dir#"$SOURCE_DIR"/}"
    mmd -i "$IMAGE" "::/$rel"
done < <(find "$SOURCE_DIR" -mindepth 1 -type d -print0 | LC_ALL=C sort -z)

# Copy files into the FAT16 image.
while IFS= read -r -d '' file; do
    rel="${file#"$SOURCE_DIR"/}"
    mcopy -i "$IMAGE" "$file" "::/$rel"
done < <(find "$SOURCE_DIR" -mindepth 1 -type f -print0 | LC_ALL=C sort -z)
