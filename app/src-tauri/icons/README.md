# Native icons

The committed icons come from `desktop/assets/icon.png`. Do not regenerate them
as part of an engine change. On macOS, use system tools in a temporary folder:

```sh
mkdir -p work/icons/HWP.iconset
sips -z 32 32 desktop/assets/icon.png --out work/icons/32x32.png
sips -z 128 128 desktop/assets/icon.png --out work/icons/128x128.png
sips -z 256 256 desktop/assets/icon.png --out work/icons/128x128@2x.png
sips -z 512 512 desktop/assets/icon.png --out work/icons/icon.png
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" desktop/assets/icon.png --out "work/icons/HWP.iconset/icon_${size}x${size}.png"
  retina=$((size * 2))
  sips -z "$retina" "$retina" desktop/assets/icon.png --out "work/icons/HWP.iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns work/icons/HWP.iconset -o work/icons/icon.icns
```

Review the temporary PNGs and ICNS before copying them here. The macOS package
uses `icon.icns`; keep the existing `icon.ico` unless a separate Windows icon
update is requested. The source SVG is `desktop/assets/icon.svg`.
