# Model resource visual QA

Issue #11 uses a compile-time-only state gallery to inspect operational states without downloading
the 16.95 GB model or exposing production model/path controls.

## Reproduction

Build the isolated QA package:

```sh
VITE_PIU_MODEL_QA_GALLERY=1 npm run build
```

Vite resolves `#model-resource-qa` to `model-resource-qa.gallery.tsx` only for that build. Normal
builds resolve the import to a zero-content production entry, and the bundle check scans emitted
JavaScript, source maps, and the Vite manifest for gallery module names and fixture strings. Each
panel receives a deterministic `statusOverride`, skips the Tauri status command and subscription,
and therefore cannot start a download or mutate model storage during inspection.
Download, cancellation, authentication, resume, and confirmation controls are disabled in QA mode;
only opening, closing, and keyboard-navigating the local removal dialogs remains interactive. A
frontend test clicks the gallery controls and asserts that no platform function is called.
The normal release build omits the variable and renders the production IPC-backed panel. The QA
context toggle is local-only and renders the same fixtures either in Settings or through the
reusable `OnboardingModelResourceStep` shell; it never changes application or model state.

The arm64 `.app` was opened at its configured 1180 × 760 window size. The native macOS window
captures include the one-point title-bar extent and are committed as 2360 × 1522, 2× RGBA PNGs.
macOS accessibility and pixel output were inspected for these states in both system appearances:

- download progress;
- integrity verification;
- Hugging Face authentication required;
- insufficient-space failure;
- cancellation with resumable partial;
- ready;
- ready-state removal dialog;
- old-revision mismatch;
- old-revision ownership-safe recovery dialog.

The same seven operational panels were exposed through the onboarding shell. Computer Use
confirmed all seven `Local model onboarding` accessibility regions in both appearances, including
the progress, verification, authentication, disk failure, cancellation, ready, and revision
mismatch states. Representative top and recovery pixels are committed below. No model request was
made; the test also asserts that every production platform mock remains untouched.

The removal dialogs exposed `aria-modal`, their labelled heading and copy, `Keep model` as initial
focus, and `Confirm removal`. The system Light appearance was restored after capture.

## Evidence

- [Light Settings progress and metrics](evidence/model-assets-settings-light-top.png)
- [Light Settings ready and old-revision recovery](evidence/model-assets-settings-light-recovery.png)
- [Dark Settings progress and metrics](evidence/model-assets-settings-dark-top.png)
- [Dark Settings ready and old-revision recovery](evidence/model-assets-settings-dark-recovery.png)
- [Light onboarding progress](evidence/model-assets-onboarding-light-top.png)
- [Light onboarding old-revision recovery](evidence/model-assets-onboarding-light-recovery.png)
- [Dark onboarding progress](evidence/model-assets-onboarding-dark-top.png)
- [Dark onboarding old-revision recovery](evidence/model-assets-onboarding-dark-recovery.png)
- [Light ready-state removal dialog](evidence/model-assets-ready-dialog-light.png)
- [Dark ready-state removal dialog](evidence/model-assets-ready-dialog-dark.png)
- [Light old-revision removal dialog](evidence/model-assets-mismatch-dialog-light.png)
- [Dark old-revision removal dialog](evidence/model-assets-mismatch-dialog-dark.png)

## Contrast

Light-appearance danger text is `#b23b36`.

- Against the light stage `#fbfcfd`, WCAG relative luminance gives `(0.976 + 0.05) / (0.129 +
  0.05) = 5.72:1`.
- Against the former 6% danger-tinted surface (`#f7f0f1`), it gives `5.23:1`.

Both exceed the WCAG AA 4.5:1 requirement for small text. The shipped dialog now uses the higher
contrast untinted stage background.
