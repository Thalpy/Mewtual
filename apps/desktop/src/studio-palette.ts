import type { PixPaletteEntry } from "./studio-contract.ts";

/** Display indices only: sorting the palette itself would recolour existing encoded pixels. */
export function paletteGroups(palette: readonly PixPaletteEntry[]): { grayscale: number[]; colors: number[]; themed: number[] } {
  const grayscale: number[] = [], colors: number[] = [], themed: number[] = [];
  palette.forEach((entry, index) => {
    if (entry.role) themed.push(index);
    else if (index < 2 || (entry.r === entry.g && entry.g === entry.b)) grayscale.push(index);
    else colors.push(index);
  });
  const lightness = (i: number) => palette[i].r * 299 + palette[i].g * 587 + palette[i].b * 114;
  grayscale.sort((a, b) => lightness(b) - lightness(a) || a - b);
  return { grayscale, colors, themed };
}

export function paletteColorLabel(entry: PixPaletteEntry): string {
  return `#${[entry.r, entry.g, entry.b].map(value => value.toString(16).padStart(2, "0")).join("")}`;
}
