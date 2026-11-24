// Open state of the command palette, shared by the shell (button, Ctrl/Cmd+K) and the palette.

class PaletteStore {
  open = $state(false);

  show(): void {
    this.open = true;
  }

  hide(): void {
    this.open = false;
  }

  toggle(): void {
    this.open = !this.open;
  }
}

export const palette = new PaletteStore();
