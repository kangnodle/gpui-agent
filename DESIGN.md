# Design System: MiniTranslator (Windows 11 Fluent × Spotlight Utility)

## 1. Visual Theme & Atmosphere

MiniTranslator is a tray-resident Windows 11 desktop utility that fuses **Windows 11 Fluent Design (WinUI 3 design language)** with the form factor and immediacy of a **Spotlight / Command Palette floating launcher**. Rather than adopting third-party UI kits, it builds directly on the **.NET 11 WPF built-in Fluent theme (`ThemeMode="System"`)** and **Win32 Desktop Window Manager (DWM)** APIs so the window feels like a native first-party Windows 11 companion tool.

- **Aesthetic Philosophy**: Quiet, tactile, translucent utility. Chrome recedes into the desktop via Windows 11 Transient Acrylic (`DWMSBT_TRANSIENTWINDOW`), rounded window geometry (`DWMWCP_ROUND`), and borderless text canvases, while interactive controls surface as crisp pill containers and subtle hairline-bordered cards.
- **Interaction Personality**: Keyboard-first, instant-on, instant-dismiss. Invoked from anywhere via a global hotkey, anchored at the upper-third center of the screen, always-on-top by default, and dismissed without ceremony via `Esc`, focus loss, or the close glyph—returning silently to the system tray.
- **Spatial Architecture**: Single-window discipline. Settings do not spawn detached modal dialogs; they transition inline inside the same window bounds, preserving translation text, cursor selection, and scroll offsets across navigation.
- **Visual Density**: Compact desktop utility density (`14 DIP` base, `15 DIP` reading/writing canvases, `48 DIP` integrated chrome bar) that stays legible across both Korean (`ko-KR`) and English (`en-US`) UI locales and Per-Monitor V2 high-DPI displays.

---

## 2. Color Palette & Roles

All views consume **WinUI 3 / WPF Fluent semantic resource keys** (`{DynamicResource ...}`) rather than raw hex values, enabling instant runtime switching across 6 themes without rebuilding the visual tree.

### Semantic Token Roles (WPF Fluent Theme Keys)

| Role | WPF Resource Key(s) | Functional Purpose |
| --- | --- | --- |
| **Window Surface** | `AppSurfaceBrush` | `Transparent` on System/Light/Dark so DWM Acrylic shows through; bound to `WindowBackground` on custom solid themes. |
| **Base / Solid Canvas** | `SolidBackgroundFillColorBaseBrush`, `WindowBackground` | Opaque backing for floating popups (`EnginePopupSurface`) and solid custom themes. |
| **Card / Elevated Layer** | `CardBackgroundFillColorDefaultBrush`, `LayerFillColorDefaultBrush` | Result pane differentiation, Settings cards (`CardStyle`), and sticky bottom bars. |
| **Control Fill** | `ControlFillColorDefaultBrush`, `ControlFillColorSecondaryBrush`, `ControlFillColorTertiaryBrush` | Pill containers, engine toggle, secondary pill buttons, keycaps, theme swatches, and hover/pressed fill progression. |
| **Subtle Interactive Fill** | `SubtleFillColorSecondaryBrush`, `SubtleFillColorTertiaryBrush` | Transparent icon buttons, ghost buttons, and flyout preset menu items on hover/press. |
| **Primary & Secondary Text** | `TextFillColorPrimaryBrush`, `TextFillColorSecondaryBrush`, `TextFillColorDisabledBrush` | Primary content and headings vs. placeholders, metadata, section headers, and shortcut hints. |
| **Accent & On-Accent** | `AccentFillColorDefaultBrush`, `AccentTextFillColorPrimaryBrush`, `AccentButtonBackground`, `AccentButtonForeground`, `TextOnAccentFillColorPrimaryBrush` | Primary CTA pills (Translate, Save), active pin/swap glyphs, checked toggle tracks, selected theme swatch border, and text selection. |
| **Borders & Hairlines** | `DividerStrokeColorDefaultBrush`, `CardStrokeColorDefaultBrush`, `ControlStrokeColorDefaultBrush`, `ControlStrongStrokeColorDefaultBrush`, `SurfaceStrokeColorFlyoutBrush` | 1px structural dividers, card outlines, pill borders, toggle tracks, and popup rims. |
| **Focus Ring** | `FocusStrokeColorOuterBrush` | 2px keyboard focus indicator around custom-templated Spotlight buttons and swatches. |
| **Critical / Error** | `SystemFillColorCriticalBrush`, `SystemFillColorCriticalBackgroundBrush` | Inline error InfoBars (`ErrorInfoBarStyle`), critical glyphs (`&#xEB90;`), and validation errors. |
| **Success** | `SystemFillColorSuccessBrush`, `SystemFillColorSuccessBackgroundBrush` | Copied confirmation checkmark (`&#xE73E;`) and positive state feedback. |

### Fixed Utility Chrome Tokens (`App.xaml`)

- **Destructive Close Hover**: `#C42B1C` (`CloseButtonHoverBackground`) with `#FFFFFF` (`CloseButtonHoverForeground`) — standard Windows 11 caption close semantics.
- **Destructive Close Pressed**: `#B02315` (`CloseButtonPressedBackground`) with `#FFFFFF` foreground.
- **Popup Drop Shadow**: `#000000` at `0.2` opacity (`EnginePopupShadowEffect`).

### Built-In System / Light / Dark Reference Swatches (`SettingsView.xaml`)

| Theme | Base ThemeMode | DWM Backdrop | Swatch Surface | Swatch Card | Swatch Accent | Swatch Border |
| --- | --- | --- | --- | --- | --- | --- |
| **System** | `ThemeMode.System` | `DWMSBT_TRANSIENTWINDOW` (Acrylic) | `#F3F3F3` / `#202020` split | `#2D2D2D` | `#0067C0` | `#33808080` |
| **Light** | `ThemeMode.Light` | `DWMSBT_TRANSIENTWINDOW` (Acrylic) | `#F3F3F3` | `#FFFFFF` | `#0067C0` | `#22000000` |
| **Dark** | `ThemeMode.Dark` | `DWMSBT_TRANSIENTWINDOW` (Acrylic) | `#202020` | `#2D2D2D` | `#60CDFF` | `#26FFFFFF` |

### Custom Solid Theme Palettes (`ThemeResources.cs`)

Every custom theme maps all Fluent control keys onto a curated 15-slot palette, switches DWM backdrop to `DWMSBT_NONE`, and is regression-tested to maintain **>= 4.5:1 WCAG AA contrast** across primary text, secondary text, interactive controls, accent buttons, error bars, and success indicators.

| Token Slot | Tomorrow Night Blue (Dark) | Solarized Light (Light) | Solarized Dark (Dark) |
| --- | --- | --- | --- |
| `Background` | `#002451` | `#FDF6E3` | `#002B36` |
| `Card` | `#001C40` | `#EEE8D5` | `#073642` |
| `Input` | `#001733` | `#FDF6E3` | `#002B36` |
| `Hover` | `#00346E` | `#E4DDC8` | `#0B3C48` |
| `Pressed` | `#003F8E` | `#DFCA88` | `#073642` |
| `Foreground` | `#FFFFFF` | `#42585E` | `#A6B4B4` |
| `Secondary` | `#BBDAFF` | `#556970` | `#93A1A1` |
| `Disabled` | `#7086A5` | `#93A1A1` | `#586E75` |
| `Border` | `#404F7D` | `#B49471` | `#586E75` |
| `Accent` | `#BBDAFF` | `#268BD2` | `#268BD2` |
| `OnAccent` | `#002451` | `#00151A` | `#00151A` |
| `Error` / `ErrorBackground` | `#FF9DA4` / `#40243E` | `#B82724` / `#FCE4D6` | `#FF6C6B` / `#412F35` |
| `Success` / `SuccessBackground` | `#D1F1A9` / `#1B3B2B` | `#2B6A00` / `#E3ECD6` | `#93C43A` / `#153D32` |

---

## 3. Typography Rules

### Font Families

- **Default UI Font (`AppFont.System`)**: `SystemFonts.MessageFontFamily` (Windows 11 `Segoe UI Variable` with system Korean fallback).
- **Selectable CJK / Korean Font Options**:
  - `AppFont.MalgunGothic`: `Malgun Gothic, Global User Interface`
  - `AppFont.Batang`: `Batang, Global User Interface`
  - `AppFont.Gulim`: `Gulim, Global User Interface`
  - `AppFont.NotoSansKR`: `Noto Sans KR, Global User Interface`
- **Icon Font (`FluentIconFont`)**: `Segoe Fluent Icons, Segoe MDL2 Assets`
- **Locale Binding**: Root `Window` and `SettingsView` bind `Language="{DynamicResource UiCulture}"` (`ko-KR` or `en-US`) so OpenType/glyph shaping and CJK fallback match the active UI language.

### Modular Type Scale (DIP)

| Role | Style / Control | Size (DIP) | Weight | Color Token | Usage |
| --- | --- | --- | --- | --- | --- |
| **Page Title** | `PageTitleStyle` | `18` | `SemiBold` | `TextFillColorPrimaryBrush` | Primary page headings when needed |
| **View Header** | `SettingsHeading` | `15` | `SemiBold` | `TextFillColorPrimaryBrush` | Integrated 48px top bar title (`Settings`) |
| **Reading / Writing Canvas** | `SourceTextBox`, `ResultTextBox`, Placeholders, `AppFontPreview` | `15` | `Regular` | Primary (text) / Secondary (placeholder) | Translation source & result editors for comfortable reading |
| **Base Body** | Window / UserControl default | `14` | `Regular` | `TextFillColorPrimaryBrush` | Standard Fluent controls (ComboBoxes, Labels, Inputs) |
| **Card Title** | `CardTitleStyle` | `13.5` | `Medium` | `TextFillColorPrimaryBrush` | Setting card primary labels |
| **Section Header** | `SectionHeaderStyle` | `13` | `SemiBold` | `TextFillColorSecondaryBrush` | Group headings in Settings (`Margin="2,18,0,6"`) |
| **Body Strong** | `BodyStrongTextStyle` | `13` | `SemiBold` | `TextFillColorPrimaryBrush` | Emphasized inline body labels |
| **Primary Pill CTA / Preset** | `SpotlightPillAccentButtonStyle`, Preset titles | `12.5` | `SemiBold` / `Medium` | `AccentButtonForeground` / Primary | Primary pill buttons (`Translate`, `Save`) and engine preset names |
| **Secondary / Chrome Controls** | `SecondaryTextStyle`, `SpotlightLanguageComboBoxStyle`, `SpotlightEnginePillStyle`, `SpotlightPillButtonStyle`, `SpotlightGhostButtonStyle`, Toggle state text | `12` | `Regular` / `Medium` | Secondary (descriptions/hints) / Primary (actions) | Helper hints, character counter, status info, pill controls |
| **Theme Swatch Caption** | Swatch labels (`SettingsView`) | `11.5` | `Regular` | `TextFillColorPrimaryBrush` | Truncated (`CharacterEllipsis`) labels under 3-column swatches |
| **Keycap / Micro Meta** | `KeycapTextStyle`, Popup header, Footer labels, Preset subtitles | `11` | `Regular` (`SemiBold` for popup title) | `TextFillColorSecondaryBrush` | Shortcut keycaps, footer hints, preset descriptions |

### Standard Icon Glyph Vocabulary (`Segoe Fluent Icons`)

| Action / Concept | Glyph Code | Size (DIP) |
| --- | --- | --- |
| Swap languages | `&#xE8AB;` | `12` (pill) / `15` (card) |
| Quick engine indicator | `&#xE945;` | `11` |
| Dropdown chevron | `&#xE70D;` | `9` |
| Selected / Copied check | `&#xE73E;` | `12` |
| Pin (unpinned / pinned) | `&#xE718;` / `&#xE840;` | `13` |
| Settings | `&#xE713;` | `14` |
| Close chrome | `&#xE8BB;` | `11` |
| Back navigation | `&#xE72B;` | `14` |
| Translate action arrow | `&#xE751;` | `10` |
| Copy / Selection capture | `&#xE8C8;` | `12` / `15` |
| Error InfoBar icon | `&#xEB90;` | `15` |
| Theme / Hotkeys / Reset / Languages / Power / Sync / Hide | `&#xE790;` / `&#xE765;` / `&#xE777;` / `&#xF2B7;` / `&#xE7E8;` / `&#xE895;` / `&#xE73F;` | `15` |

---

## 4. Component Stylings

### Integrated Spotlight Top Bar (`48 DIP` height)

- Separated from content by a `1px` bottom hairline (`DividerStrokeColorDefaultBrush`), `Padding="16,0,12,0"` (Translator) or `12,0,12,0` (Settings).
- Acts as the draggable window caption via `WindowChrome.CaptionHeight="48"`. Every interactive child inside the top bar must declare `WindowChrome.IsHitTestVisibleInChrome="True"`.
- Houses a left-aligned **Language Pill Container** (`SpotlightPillContainerStyle`: `CornerRadius="16"`, `Padding="4,2"`, 1px border, containing two borderless `26 DIP` language ComboBoxes flanking a `26×26` circular swap button) and a right-aligned utility cluster (Engine Pill Toggle, Pin Toggle, Settings Icon Button, Close Button).

### Buttons & Toggles

| Variant | Style Key | Dimensions & Shape | Default → Hover → Pressed → Disabled |
| --- | --- | --- | --- |
| **Primary Pill CTA** | `SpotlightPillAccentButtonStyle` | `MinHeight="30"`, `CornerRadius="15"`, `Padding="16,5"` (`MinWidth="96"` / `"120"`) | `AccentButtonBackground` → `PointerOver` → `Pressed` → `Disabled` (`BorderThickness="0"`) |
| **Secondary Pill** | `SpotlightPillButtonStyle` | `MinHeight="30"`, `CornerRadius="15"`, `Padding="12,5"` | `ControlFillColorDefault` + 1px border → `Secondary` → `Tertiary` → `Opacity="0.52"` |
| **Ghost Action** | `SpotlightGhostButtonStyle` | `MinHeight="26"`, `CornerRadius="6"`, `Padding="8,3"` | `Transparent` (Secondary text) → `SubtleFillColorSecondary` (Primary text) → `Tertiary` → `Opacity="0.52"` |
| **Chrome Icon Button** | `SpotlightIconButtonStyle` | `30×30`, `CornerRadius="6"` | `Transparent` (Secondary glyph) → `SubtleFillColorSecondary` (Primary glyph) → `Tertiary` → `Opacity="0.45"` |
| **Circular Accent Icon** | `SpotlightRoundIconButtonStyle` | `26×26`, `CornerRadius="13"` | `Transparent` (`AccentTextFillColorPrimary`) → `SubtleFillColorSecondary` → `Tertiary` → `Opacity="0.45"` |
| **Pin Toggle Button** | `SpotlightPinButtonStyle` | `30×30`, `CornerRadius="6"` | Checked swaps glyph `&#xE718;` → `&#xE840;` and foreground to `AccentTextFillColorPrimaryBrush` |
| **Engine Pill Toggle** | `SpotlightEnginePillStyle` | `Height="28"`, `CornerRadius="14"`, `Padding="10,0"` | `ControlFillColorDefault` + 1px border; Checked switches border to `AccentFillColorDefaultBrush` and fill to `ControlFillColorSecondaryBrush` |
| **Close Chrome Button** | `SpotlightCloseButtonStyle` | `30×30`, `CornerRadius="6"` | `Transparent` → `#C42B1C` fill + `#FFFFFF` glyph → `#B02315` fill |
| **Standard Fluent Icon Button** | `IconButtonStyle` | `MinWidth="32"`, `MinHeight="32"`, `Padding="8"` | Inherits default WPF Fluent `Button` template (used inside Settings cards) |
| **Windows 11 Toggle Switch** | `ToggleSwitchCheckBoxStyle` | Track `40×20` (`CornerRadius="10"`), Thumb `12×12` (`13×13` on hover), preceded by `Off`/`On` text (`Margin="0,0,10,0"`) | Unchecked: transparent track + `ControlStrongStrokeColorDefaultBrush` border + left-aligned secondary thumb; Checked: `AccentFillColorDefaultBrush` track + right-aligned `TextOnAccentFillColorPrimaryBrush` thumb |

### Text Inputs & Reading Canvases

- **Borderless Translation Canvas (`SpotlightTextBoxStyle`)**: `Background="Transparent"`, `BorderThickness="0"`, `Padding="14,14,14,6"`, `MinHeight="48"`, `FontSize="15"`, `TextWrapping="Wrap"`, `AcceptsTab="False"` (so `Tab` navigates controls, never traps focus), `MaxLength="5000"`.
- **Read-Only Result Canvas**: Identical `SpotlightTextBoxStyle` with `IsReadOnly="True"` and `IsTabStop="True"` so users can select and copy partial results via keyboard. Overlaid with a `2 DIP` top-docked indeterminate `ProgressBar` while translating.
- **Placeholder Overlay**: Non-hit-testable `TextBlock` (`IsHitTestVisible="False"`) that dynamically updates copy across empty, translating, and error-recovery states.
- **Hotkey Recorder (`HotkeyBox`)**: Styled as a native Fluent `TextBox` (`IsReadOnly="True"`, IME disabled, no context menu), showing live pending modifier keys during chord press and validating modifier requirements.

### Cards, Flyouts, InfoBars & Keycaps

- **Settings Card (`CardStyle` / `SettingsCardStyle`)**: `CardBackgroundFillColorDefaultBrush`, 1px `CardStrokeColorDefaultBrush`, `CornerRadius="8"`, `Padding="14,12"`, `Margin="0,0,0,6"`. Standard internal anatomy: `[Left 15 DIP Fluent Icon (Margin 0,0,12,0) | Title (13.5 Medium) + Hint (12 Secondary) | Right Control]`.
- **Theme Swatch Picker (`ThemeSwatchItemStyle`)**: `ListBox` with `UniformGrid Columns="3"`. Each swatch card has `CornerRadius="8"`, `Padding="6"`, 1px border, containing a `28 DIP` tall mini UI preview (`CornerRadius="4"`) and `11.5 DIP` label. Selected state promotes border to `2px AccentFillColorDefaultBrush` (compensating `Padding="5"` to prevent layout shift).
- **Quick Engine Flyout Popup (`EngineQuickPopup`)**: Right-aligned custom-placed `Popup` (`AllowsTransparency="True"`, `StaysOpen="False"`). Double-border surface: outer `SolidBackgroundFillColorBaseBrush` (`CornerRadius="10"`, 1px `SurfaceStrokeColorFlyoutBrush`, `EnginePopupShadowEffect`) + inner `CardBackgroundFillColorDefaultBrush` (`CornerRadius="9"`, `Padding="6"`). Items use `SpotlightPresetItemStyle` (`CornerRadius="6"`, `Padding="10,7"`) with trailing accent checkmark.
- **Inline Error InfoBar (`ErrorInfoBarStyle`)**: `SystemFillColorCriticalBackgroundBrush`, 1px `CardStrokeColorDefaultBrush`, `CornerRadius="8"`, `Padding="12,8"`, leading `&#xEB90;` glyph + `LiveTextBlock` (`AutomationProperties.LiveSetting="Assertive"`).
- **Keycap Badge (`KeycapStyle`)**: `ControlFillColorDefaultBrush`, 1px `ControlStrokeColorDefaultBrush`, `CornerRadius="4"`, `Padding="5,1.5"`, `11 DIP` secondary text.

### Focus Visuals

- Custom templates never suppress focus indicators; they apply `SpotlightFocusVisualStyle` (`CornerRadius="8"`, `Margin="-2"`, `2px FocusStrokeColorOuterBrush`) for rectangular controls and `SpotlightPillFocusVisualStyle` (`CornerRadius="16"`) for pills, circular buttons, and toggle switches.

---

## 5. Layout Principles

### Window Dimensions & Placement

- **Default Window Size**: `580 × 560 DIP` (`MinWidth="440"`, `MinHeight="520"`, `ResizeMode="CanResize"`).
- **Initial Screen Position**: Centered horizontally in `SystemParameters.WorkArea`, anchored vertically at the **upper third** (`area.Top + Math.Max(0, (area.Height - Height) / 3)`)—mirroring Spotlight/Raycast ergonomics so the user's gaze stays near the upper-middle workspace.
- **Maximized Compensation**: When `WindowState == Maximized`, `RootChrome.Margin` switches from `0` to `8 DIP` to keep chrome aligned inside the visible monitor bounds.

### Vertical Shell Anatomy

1. **Translator View (`TranslationView`)**:
   - Row 0 (`48 DIP`): Integrated Spotlight Top Bar
   - Row 1 (`*`): Adaptive Source & Result Panels separated by a `1px` `PanelDivider`
   - Row 2 (`Auto`): Conditional `TranslationErrorBar` (`Margin="16,8,16,8"`)
   - Row 3 (`Auto`): Sticky Keycap Shortcut Footer (`Padding="16,7"`, `LayerFillColorDefaultBrush`, 1px top divider)
2. **Inline Settings View (`SettingsView` inside `SettingsHost`)**:
   - Row 0 (`48 DIP`): Back button (`&#xE72B;`), `SettingsHeading`, and Close button (`&#xE8BB;`)
   - Row 1 (`*`): `ScrollViewer` (`Padding="18,12,18,16"`) containing grouped `SectionHeaderStyle` + `SettingsCardStyle` stacks
   - Row 2 (`Auto`): Sticky Action Footer (`Padding="18,10"`, `LayerFillColorDefaultBrush`, 1px top divider) with inline `SaveErrorBar` and right-aligned `Save` (`IsDefault="True"`, `Alt+S`) + `Cancel` (`IsCancel="True"`) pill buttons (`MinWidth="120"`).

### Spacing & Radius Rhythm

- **Spacing Scale**: `2`, `4`, `6`, `8`, `10`, `12`, `14`, `16`, `18 DIP` (primary container gutters are `16 DIP` in Translator and `18 DIP` in Settings; card inter-item gap is `6 DIP`; section break top margin is `18 DIP`).
- **Radius Hierarchy**:
  - `4 DIP`: Micro elements (Keycaps, swatch inner thumbnail)
  - `6 DIP`: Flat/subtle interactive targets (Icon buttons, ghost buttons, menu items)
  - `8 DIP`: Content containers (Cards, InfoBars, Swatch tiles, standard focus rings)
  - `9–10 DIP`: Floating surfaces & toggles (Engine flyout, ToggleSwitch track)
  - `12 DIP`: Top-level window corners
  - `13–16 DIP`: Pill family (Round swap button `13`, Engine pill `14`, Action pills `15`, Language pill container & pill focus rings `16`)

### State-Preserving Inline Navigation

- Opening settings sets `TranslationView.Visibility = Visibility.Hidden` (never `Collapsed` or destroyed) and mounts `SettingsView` in `SettingsHost`. Closing settings restores exact focus target, caret selection, and `SourceTextBox`/`ResultTextBox` scroll offsets without altering window size or position.
- Translation `InputBindings` live on `TranslationView` (not `Window`) so translation shortcuts cannot fire while Settings is active.

---

## 6. Depth & Elevation

MiniTranslator uses **tonal surface stepping + 1px hairlines + DWM backdrop** rather than heavy drop shadows.

| Elevation Layer | Surface Implementation | Border / Shadow Treatment |
| --- | --- | --- |
| **Layer 0 — Window Backdrop** | `AppSurfaceBrush` (`Transparent` + DWM `DWMSBT_TRANSIENTWINDOW` Acrylic, or solid theme `WindowBackground`) | Native Windows 11 OS window shadow and `DWMWCP_ROUND` corners |
| **Layer 1 — Secondary / Footer / Result** | `CardBackgroundFillColorDefaultBrush` (Result panel, Settings cards) & `LayerFillColorDefaultBrush` (Bottom shortcut/action bars) | `1px DividerStrokeColorDefaultBrush` or `1px CardStrokeColorDefaultBrush`; zero drop shadows |
| **Layer 2 — Interactive Controls** | `ControlFillColorDefaultBrush` (Pills, secondary buttons, swatches, keycaps) | `1px ControlStrokeColorDefaultBrush`; accent border (`1px` or `2px`) on active/selected states |
| **Layer 3 — Floating Flyout** | `EngineQuickPopup` (`SolidBackgroundFillColorBaseBrush` outer + `CardBackgroundFillColorDefaultBrush` inner) | `1px SurfaceStrokeColorFlyoutBrush` + `DropShadowEffect` (`BlurRadius="12"`, `ShadowDepth="4"`, `Opacity="0.2"`, `Color="#000000"`, `RenderingBias="Performance"`) |

---

## 7. Do's and Don'ts

### Do

- **Do** bind every color, stroke, and text brush via `{DynamicResource ...}` using WinUI 3 / WPF Fluent token names so all 6 themes (including custom solid palettes) update live.
- **Do** keep `AppSurfaceBrush` transparent on System/Light/Dark themes so Windows 11 Acrylic renders seamlessly behind the source pane and top bar.
- **Do** use pill shapes (`CornerRadius="14–16"`) exclusively for Spotlight chrome pills and primary/secondary action buttons, and `CornerRadius="8"` for cards and InfoBars.
- **Do** preserve keyboard-first ergonomics: assign explicit `TabIndex`, provide `AutomationProperties.Name/HelpText/AcceleratorKey`, and route status/error updates through `LiveTextBlock` (`Polite` for progress/copy, `Assertive` for errors).
- **Do** handle `Esc` in strict layered order: close open dropdowns/popups first → close inline Settings → hide window to tray.
- **Do** verify `>= 4.5:1` contrast whenever adding or modifying custom theme palettes.
- **Do** keep settings inline inside `SettingsHost` and keep Save/Cancel pinned outside the `ScrollViewer`.

### Don't

- **Don't** introduce third-party WPF theme packs (MahApps, MaterialDesign, WPF-UI) or WinUI 3 / Windows App SDK packages; stay on built-in .NET WPF Fluent + Win32 DWM.
- **Don't** hardcode light/dark hex colors in views (only fixed Windows close red `#C42B1C`/`#B02315`, theme preview swatches, and `ThemeResources.cs` palette definitions may carry hex literals).
- **Don't** add heavy box shadows to cards, panels, or buttons—reserve `DropShadowEffect` solely for floating `Popup` surfaces (`EngineQuickPopup`).
- **Don't** spawn secondary windows or modal dialogs for settings or quick engine selection.
- **Don't** trap `Tab` inside multiline translation inputs (`AcceptsTab="False"` is mandatory) or allow translation shortcuts to trigger behind inline Settings.
- **Don't** forget `WindowChrome.IsHitTestVisibleInChrome="True"` on any interactive control placed inside the `48 DIP` top bar.
- **Don't** terminate the process on window close; hide to `Win32TrayIcon` and let `AppShell` enter Windows Efficiency Mode after idle hide (`15s`).

---

## 8. Responsive Behavior

As a resizable desktop utility (`MinWidth="440"`, `MinHeight="520"`), MiniTranslator adapts continuously across three width breakpoints without clipping or horizontal scrollbars:

| Breakpoint (DIP) | Affected Surface | Narrow Behavior | Wide Behavior |
| --- | --- | --- | --- |
| **`720 DIP`** | `TranslationPanels` (`TranslatorWindow.xaml.cs`) | Vertical 2-row stack: Source (Row 0) → 1px horizontal `PanelDivider` (Row 1) → Result (Row 2) | Side-by-side 2-column split: Source (Col 0) → 1px vertical `PanelDivider` (Col 1) → Result (Col 2), equal `*` widths (`>= 280 DIP` each at `900 DIP`) |
| **`500 DIP`** | `TopBar` language selectors (`TranslatorWindow.xaml.cs`) | `SourceLanguageBox` and `TargetLanguageBox` `MaxWidth` scales smoothly via `Math.Max(64, (width - 266) / 2)` so the language pill never collides with `EngineQuickButton` | Fixed `MaxWidth="140"` per language ComboBox |
| **`440 DIP`** | `EngineToolbar` in Settings (`SettingsView.xaml.cs`) | Collapses from 1 row into 3 vertical rows (Engine / Model / Effort) with help `?` button anchored in Column 2 | Single horizontal 7-column inline toolbar (`0.85*` Engine \| `1.2*` Model \| `1*` Effort \| `?`) |

### Minimum Targets & Accessibility Guarantees

- Chrome icon buttons: `30×30 DIP` (`26×26 DIP` inside the language pill); standard Settings icon buttons: `>= 32×32 DIP`; primary Settings footer buttons: `>= 120 DIP` wide.
- Both `SourceTextBox` and `ResultTextBox` guarantee `ActualHeight >= 48 DIP` even at minimum window size (`440×520`) while `TranslationErrorBar` is visible.
- Full bilingual expansion (`ko-KR` and `en-US`) is regression-tested at minimum dimensions in `WindowUxTests.cs`.

---

## 9. Agent Prompt Guide

Use this reference when generating or modifying WPF XAML views, styles, or desktop interactions in MiniTranslator.

### Quick Token Reference

- **Theme Engine**: `ThemeMode="System"` in `App.xaml` + `WindowBackdrop.ApplyAcrylic(window, theme)` (`DWMSBT_TRANSIENTWINDOW` / `DWMSBT_NONE` + `DWMWCP_ROUND` + `DWMWA_USE_IMMERSIVE_DARK_MODE`).
- **Key Brushes**: `AppSurfaceBrush`, `CardBackgroundFillColorDefaultBrush`, `LayerFillColorDefaultBrush`, `ControlFillColorDefaultBrush`, `SubtleFillColorSecondaryBrush`, `DividerStrokeColorDefaultBrush`, `CardStrokeColorDefaultBrush`, `TextFillColorPrimaryBrush`, `TextFillColorSecondaryBrush`, `AccentFillColorDefaultBrush`, `AccentButtonBackground`, `SystemFillColorCriticalBrush`, `SystemFillColorCriticalBackgroundBrush`, `SystemFillColorSuccessBrush`.
- **Key Styles**: `SpotlightPillContainerStyle`, `SpotlightLanguageComboBoxStyle`, `SpotlightEnginePillStyle`, `SpotlightIconButtonStyle`, `SpotlightRoundIconButtonStyle`, `SpotlightPinButtonStyle`, `SpotlightCloseButtonStyle`, `SpotlightPillAccentButtonStyle`, `SpotlightPillButtonStyle`, `SpotlightGhostButtonStyle`, `SpotlightTextBoxStyle`, `SpotlightPresetItemStyle`, `CardStyle`, `SettingsCardStyle`, `ErrorInfoBarStyle`, `KeycapStyle`, `KeycapTextStyle`, `ToggleSwitchCheckBoxStyle`, `IconStyle`, `SectionHeaderStyle`, `SecondaryTextStyle`.
- **Fonts**: `{DynamicResource AppFontFamily}` for UI text, `{StaticResource FluentIconFont}` (`Segoe Fluent Icons, Segoe MDL2 Assets`) for glyphs.
- **Shortcuts**: Global hotkey (default `Ctrl+Space`) toggles/captures; `Ctrl+Enter` translate; `Ctrl+Shift+S` swap; `Ctrl+Shift+C` copy; `Ctrl+,` settings; `Alt+S` save settings; `Esc` layered dismiss.

### Prompt Blueprint for New UI Features

> Build the feature in `apps/MiniTranslator.Wpf` strictly following MiniTranslator's **Windows 11 Fluent × Spotlight Utility** design system:
> 1. Use only `.NET 11 WPF` built-in Fluent controls and `{DynamicResource ...}` Fluent brush tokens so System, Light, Dark, Tomorrow Night Blue, Solarized Light, and Solarized Dark all work automatically with >= 4.5:1 contrast.
> 2. Keep all flows inside `TranslatorWindow` (inline views or lightweight `Popup` flyouts) without creating new top-level windows; preserve translation text, selection, scroll offsets, and window bounds.
> 3. Follow the Spotlight geometry (`48 DIP` integrated chrome with `WindowChrome.IsHitTestVisibleInChrome="True"`, `CornerRadius="14–16"` pills, `CornerRadius="8"` cards with `1px` borders, `CornerRadius="6"` icon/ghost buttons, `Segoe Fluent Icons` glyphs, and `SpotlightFocusVisualStyle`/`SpotlightPillFocusVisualStyle`).
> 4. Ensure keyboard-first navigation (`TabIndex`, `InputBindings`, layered `Esc`), full `ko-KR`/`en-US` localization via `{DynamicResource ...}`, `AutomationProperties`, and `LiveTextBlock` live regions.
> 5. Verify layout at `440×520`, `580×560`, and `900×520` (respecting the `440` / `500` / `720 DIP` breakpoints) and add STA UI assertions to `tests/MiniTranslator.Wpf.Tests/WindowUxTests.cs`.
