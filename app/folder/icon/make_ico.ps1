# Fabrique folder.ico (ressource 1 de l'exécutable, icône de l'installeur) à partir de
# icone.png, sans dépendance (System.Drawing) :
#
#   powershell -ExecutionPolicy Bypass -File app\folder\icon\make_ico.ps1
#
# 1. Nettoyage : l'image source porte des pixels parasites dans sa zone transparente (restes
#    de détourage) ; seule la plus grande zone opaque (le dossier) est gardée, avec une
#    bordure de 3 px pour son anticrénelage.
# 2. Recadrage carré autour du dossier (marge de 2 %).
# 3. Réduction à chaque taille en alpha prémultiplié (pas de halo sombre sur les bords).
# 4. .ico aux entrées PNG (acceptées par Windows depuis Vista), de 256 à 16 px ;
#    folder-256.png pour le README ; petite image de l'assistant d'installation.
param(
    [string]$Source = "$PSScriptRoot\icone.png",
    [string]$Ico = "$PSScriptRoot\..\folder.ico",
    [string]$Preview = "$PSScriptRoot\folder-256.png",
    # Petite image de l'assistant d'installation (Inno Setup), 100 % et 200 %.
    [string]$Wizard = "$PSScriptRoot\..\..\..\installer",
    # Dossier où écrire chaque taille en PNG (contrôle visuel), facultatif.
    [string]$Dump = ""
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class FolderIcon {
    // Garde la plus grande composante (alpha > seuil, 8-connexité) dilatée de `grow` px.
    public static Bitmap Clean(Bitmap src, int threshold, int grow) {
        int w = src.Width, h = src.Height;
        var px = Read(src);
        var label = new int[w * h];
        int best = 0, bestSize = 0, next = 0;
        var stack = new Stack<int>();
        for (int i = 0; i < w * h; i++) {
            if (label[i] != 0 || Alpha(px, i) <= threshold) continue;
            next++; int size = 0; label[i] = next; stack.Push(i);
            while (stack.Count > 0) {
                int p = stack.Pop(); size++;
                int x = p % w, y = p / w;
                for (int dy = -1; dy <= 1; dy++) for (int dx = -1; dx <= 1; dx++) {
                    int nx = x + dx, ny = y + dy;
                    if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
                    int q = ny * w + nx;
                    if (label[q] == 0 && Alpha(px, q) > threshold) { label[q] = next; stack.Push(q); }
                }
            }
            if (size > bestSize) { bestSize = size; best = next; }
        }
        var keep = new bool[w * h];
        for (int i = 0; i < w * h; i++) keep[i] = label[i] == best;
        for (int g = 0; g < grow; g++) {
            var grown = (bool[])keep.Clone();
            for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
                if (keep[y * w + x]) continue;
                for (int dy = -1; dy <= 1 && !grown[y * w + x]; dy++) for (int dx = -1; dx <= 1; dx++) {
                    int nx = x + dx, ny = y + dy;
                    if (nx >= 0 && ny >= 0 && nx < w && ny < h && keep[ny * w + nx]) { grown[y * w + x] = true; break; }
                }
            }
            keep = grown;
        }
        for (int i = 0; i < w * h; i++) if (!keep[i]) { px[i * 4] = 0; px[i * 4 + 1] = 0; px[i * 4 + 2] = 0; px[i * 4 + 3] = 0; }
        return Write(px, w, h);
    }

    // Carré englobant les pixels visibles, avec une marge relative.
    public static Rectangle Bounds(Bitmap src, double margin) {
        int w = src.Width, h = src.Height;
        var px = Read(src);
        int x0 = w, y0 = h, x1 = -1, y1 = -1;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (Alpha(px, y * w + x) > 0) {
            if (x < x0) x0 = x; if (y < y0) y0 = y; if (x > x1) x1 = x; if (y > y1) y1 = y;
        }
        int side = Math.Max(x1 - x0 + 1, y1 - y0 + 1);
        side += (int)(side * margin * 2);
        int cx = (x0 + x1) / 2, cy = (y0 + y1) / 2;
        return new Rectangle(cx - side / 2, cy - side / 2, side, side);
    }

    // Réduction de qualité, en alpha prémultiplié.
    public static Bitmap Resize(Bitmap src, Rectangle area, int size) {
        var pre = new Bitmap(src.Width, src.Height, PixelFormat.Format32bppPArgb);
        using (var g = Graphics.FromImage(pre)) g.DrawImage(src, 0, 0, src.Width, src.Height);
        var dst = new Bitmap(size, size, PixelFormat.Format32bppPArgb);
        using (var g = Graphics.FromImage(dst))
        using (var attrs = new ImageAttributes()) {
            g.Clear(Color.Transparent);
            g.CompositingMode = CompositingMode.SourceCopy;
            g.InterpolationMode = InterpolationMode.HighQualityBicubic;
            g.PixelOffsetMode = PixelOffsetMode.HighQuality;
            g.SmoothingMode = SmoothingMode.HighQuality;
            attrs.SetWrapMode(WrapMode.TileFlipXY);
            g.DrawImage(pre, new Rectangle(0, 0, size, size), area.X, area.Y, area.Width, area.Height, GraphicsUnit.Pixel, attrs);
        }
        pre.Dispose();
        var outp = new Bitmap(size, size, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(outp)) { g.CompositingMode = CompositingMode.SourceCopy; g.DrawImage(dst, 0, 0, size, size); }
        dst.Dispose();
        return outp;
    }

    static int Alpha(byte[] px, int i) { return px[i * 4 + 3]; }

    static byte[] Read(Bitmap b) {
        var data = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        var px = new byte[b.Width * b.Height * 4];
        for (int y = 0; y < b.Height; y++) Marshal.Copy(data.Scan0 + y * data.Stride, px, y * b.Width * 4, b.Width * 4);
        b.UnlockBits(data);
        return px;
    }

    static Bitmap Write(byte[] px, int w, int h) {
        var b = new Bitmap(w, h, PixelFormat.Format32bppArgb);
        var data = b.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
        for (int y = 0; y < h; y++) Marshal.Copy(px, y * w * 4, data.Scan0 + y * data.Stride, w * 4);
        b.UnlockBits(data);
        return b;
    }
}
"@

$sizes = 256, 128, 96, 64, 48, 40, 32, 24, 20, 16
$src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Source))
$clean = [FolderIcon]::Clean($src, 40, 3)
$src.Dispose()
$area = [FolderIcon]::Bounds($clean, 0.02)

$pngs = @()
foreach ($s in $sizes) {
    $img = [FolderIcon]::Resize($clean, $area, $s)
    $ms = New-Object System.IO.MemoryStream
    $img.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    if ($s -eq 256) { $img.Save($Preview, [System.Drawing.Imaging.ImageFormat]::Png) }
    if ($Dump) { $img.Save((Join-Path $Dump "icon-$s.png"), [System.Drawing.Imaging.ImageFormat]::Png) }
    $img.Dispose()
    $pngs += , @($s, $ms.ToArray())
}
foreach ($s in 55, 110) {
    # BMP 32 bits avec canal alpha (WizardImageAlphaFormat=defined dans folder.iss).
    $img = [FolderIcon]::Resize($clean, $area, $s)
    $img.Save((Join-Path (Resolve-Path $Wizard) "wizard-small-$s.bmp"), [System.Drawing.Imaging.ImageFormat]::Bmp)
    $img.Dispose()
}
$clean.Dispose()

# En-tête ICONDIR, une entrée ICONDIRENTRY par taille, puis les PNG.
$out = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter($out)
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$pngs.Count)
$offset = 6 + 16 * $pngs.Count
foreach ($p in $pngs) {
    $dim = if ($p[0] -ge 256) { 0 } else { $p[0] }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$p[1].Length); $w.Write([uint32]$offset)
    $offset += $p[1].Length
}
foreach ($p in $pngs) { $w.Write([byte[]]$p[1]) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path (Resolve-Path (Split-Path $Ico)) (Split-Path $Ico -Leaf)), $out.ToArray())
"{0} : {1} tailles, {2} octets ; carré source {3} px" -f $Ico, $pngs.Count, $out.Length, $area.Width
