## Rust Server for the Brood War League Meta Feature

- Initially available only for admin
- Takes YouTube link to SOOP Duel Series, KCM Race Survival, and ASL matches
- Extracts minerals, gas, supply per player every 5 seconds via OCR
- Store in AWS
- Use existing Chart.js style to display in Meta tab in Brood War League
- Auto add charts to website as part of pipeline
- Concurrent job handling
