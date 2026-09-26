# VRC Live Caption

VRC Live Caption turns microphone speech into captions in the VRChat Chatbox.
Choose a microphone, start captioning, and let nearby players read what you say
without leaving VRChat.

> [!IMPORTANT]
> This is a beta-stage project under active development. There is no published
> release or installer yet, so trying the app currently means building it from
> source. Development and hands-on VRChat testing primarily target Windows.

## What works today

- Capture a selected microphone and check its level with a short, local-only
  microphone test.
- Recognize speech with OpenAI in either **Completed** mode (send after a speech
  unit finishes) or **Live** mode (update the caption while speech continues).
- Preview captions in the app and publish them to the VRChat Chatbox through
  OSC.
- In **Completed** mode, translate each completed caption into English (`en`)
  or Simplified Chinese (`zh-Hans`) and publish Source-only, Translation-only,
  or Bilingual captions.
- Pace, wrap, and paginate Chatbox messages around VRChat's practical limits.
- Show connection and runtime failures, reconnect from supported transient
  failures, and stop without publishing late captions.

The cloud recognition paths have been tested with a real microphone and VRChat
on Windows. Translation is implemented but has not yet completed its
authenticated provider and native VRChat validation.

## What is still being built

Live translation, local/offline recognition, a Chinese interface, and a
supported Windows installer are not available yet. See the
[roadmap](./docs/roadmap.md) for current progress and sequencing.

## Cloud uploads and credentials

The recognition paths available today use OpenAI. While cloud recognition is
active, the app uploads microphone audio to OpenAI for transcription.

When the selected caption content includes Translation, the app also sends each
completed caption's text to the selected Translation endpoint: OpenAI, or a
Custom HTTPS endpoint compatible with the OpenAI Responses API. A Custom
endpoint receives that text and its own separate credential, never microphone
audio or the OpenAI key, and its operator's retention policy is outside the
app's control. Source-only captions make no Translation requests.

You need your own OpenAI API key, plus a credential for any Custom Translation
endpoint, and are responsible for any provider usage or charges.

API keys entered in Settings are stored in the operating system credential
store and are not written to ordinary app configuration or logs. For development,
the app can also read `OPENAI_API_KEY` from the environment. The local microphone
test does not contact any cloud service.

## Try the current build from source

Install Git, the platform-specific
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), Rust through
`rustup`, and the Node/pnpm versions declared in
[`package.json`](./package.json). Then:

```sh
git clone https://github.com/Harry-Jing/vrc-live-caption.git
cd vrc-live-caption
pnpm install --frozen-lockfile
pnpm tauri dev
```

When the app opens:

1. Enable OSC in VRChat.
2. In Settings, select and test a microphone, add an OpenAI API key, and choose
   the recognition and publication modes. To translate, choose Translation-only
   or Bilingual content, a target language, and a Translation endpoint.
3. Save the settings, return to Captioning, and start the runtime.

The app defaults to VRChat's local OSC target. Change the host or port only if
your VRChat setup uses a different target.

## Project information

- [Product direction](./docs/product.md)
- [Implementation roadmap](./docs/roadmap.md)
- [Documentation guide](./docs/README.md)
- [Contributing](./CONTRIBUTING.md)
- [Issue tracker](https://github.com/Harry-Jing/vrc-live-caption/issues)

## License

VRC Live Caption is available under the [MIT License](./LICENSE).
