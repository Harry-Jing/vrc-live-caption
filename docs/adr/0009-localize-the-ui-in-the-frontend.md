# Localize the UI in the frontend

Date: 2026-06

The UI will ship in English and Chinese. The application runtime never
localizes: it emits stable codes plus English fallback text, and the frontend
owns all user-facing presentation. Caption language, UI locale, and translation
target stay independent settings.
