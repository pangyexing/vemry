# German (`de`) translation conventions

The rules `de.json` is written to. Concept words come from
[docs/GLOSSARY.md](../../docs/GLOSSARY.md); these settle everything else two
translators would otherwise do differently. A new string follows them, and a
changed rule changes the catalog with it.

## Style

- Register: du, never Sie; du/dein/dich/dir are lower case in interface text. No „bitte“ in buttons, hints or errors (the English has no „please“ either) and no exclamation marks.
- Buttons, menu items, command titles and checkbox labels use the infinitive, object first, no article or possessive: „Notiz löschen“, „KI-Änderung zurücknehmen“, „Wissensbasis wechseln…“. Instructions in hints, placeholders, toasts and empty states use the du-imperative: „Öffne zuerst eine Wissensbasis“. A description that starts with an English third-person verb („Closes …“) stays third person („Schließt …“).
- Confirmations and status toasts end on the participle, without a subject and without „erfolgreich“: „Kopiert“, „Als geprüft markiert“, „{n} Änderungen zurückgenommen“. Errors say plainly what failed, then the cause after a colon or a dash: „Die Datei konnte nicht gelesen werden: {error}“ – no „Fehler:“ prefix, no „Leider“. Confirm dialogs ask in du form and the button names the action („Löschen bestätigen“), never „Ja“/„Nein“.
- Capitalisation: sentence case for labels, never Title Case (menu.licenses „Open-Source Licenses“ → „Open-Source-Lizenzen“). German nouns are always capitalised, even where the English label is lower case (task.field.*, chips: „Schätzung {v}“, „ID {id}“); adjectives, participles and adverbs stay lower case in inline forms (fällig, blockiert, ungeprüft, heute, erstellt am).
- Quotation marks: English “…” becomes „…“ (U+201E … U+201C), nested quotes ‚…‘ (U+201A … U+2018). Never straight "…" or »…«. Wrap {name}, {title}, {query} in „…“ exactly where the English wraps them.
- Dashes and separators: the English em dash „—“ becomes a spaced Gedankenstrich „ – “ (U+2013 with a space on both sides); ranges take an unspaced „–“ (3–5). Keep „·“, „›“ and arrows (→ ↶ ✓ ✕) exactly as in the source: „Einstellungen › Konto“.
- Ellipsis: always the single character „…“ (U+2026), attached to the word without a space, as in the source („Wissensbasis wechseln…“, „Wird geladen…“); keep it exactly where the English has it, since it signals that a further dialog or input follows.
- Compounds: German + German is written closed (Blockindex, Seitenleiste, Gedächtnisdatei, Notizverlauf); anything with the brand, an abbreviation or an English word takes hyphens (vemry-Konto, vemry-Hosting, KI-Antwort, Block-ID, Positions-ID, Clip-Ziel, Credit-Paket, Open-Source-Lizenzen). Never leave a space inside a compound („KI Änderung“ is wrong). Avoid compounds on „Wissensbasis“: „die Thesen der Wissensbasis“, „Ordner als Wissensbasis öffnen“.
- Genders, so articles agree: die Wissensbasis (Pl. Wissensbasen), der Block (Blöcke), die Notiz, der Abschnitt, das Feld, das Zustandsfeld, die Stufe, der Chip, die Projektion, die These, der Beleg, der Verweis, die Einbettung, die Privatzone, der private Block, die Gedächtnisdatei, die Berechtigungsschranke, der Skill (Skills), das Benutzerskript, das Auditprotokoll, die Herkunft, die Sitzung, die Runde, das Konto, das Kontingent, das Credit-Paket, das Gerätetoken, der Tab, der Clip, das Lesezeichen, die Aufgabe, die Karte, die KI, das Modell, der Diff, der Link, das Tag (Tags). „Credits“ is used only in the plural or with a number („1 Credit“, „{n} Credits“).
- The AI is a tool: „AI“ is always „KI“ (die KI; KI-Änderung, KI-Block, KI-Antwort). Use neutral process verbs as the English does (liest, schreibt, entwirft, durchsucht, schlägt vor); never „denkt nach“, „möchte“, „versteht“, „hilft dir gern“, no first person, no „Assistent“, no „er/sie“ for the KI – repeat „die KI“ or use the passive. The verb for the AI's writes in the gate is „schreiben“, the object a „Schreibvorgang“.
- Recurring words outside the glossary: session = Sitzung; turn = Runde; write (noun) = Schreibvorgang; diff = Diff (der); context = Kontext; model = Modell; provider = Anbieter; prompt = Prompt; token = Token; #tag = Tag (das, Pl. Tags), never „Schlagwort“; web page = Webseite; a citation in an AI answer = Quellenangabe; source (of a clip or search) = Quelle; quote = Zitat; key / API key = Schlüssel / API-Schlüssel; Allow / Deny in the permission dialog = Erlauben / Ablehnen.
- Keyboard keys: key.ctrl = Strg, key.shift = Umschalt, key.alt = Alt, key.backspace = Rücktaste, key.super = Win. In running text: Enter, Esc, Tab, Entf (Delete), Leertaste (Space), Pos1/Ende (Home/End); macOS symbols ⌘ ⌥ ⇧ stay. Combinations with „+“ and no spaces: Strg+Umschalt+P.
- Placeholders: keep every {placeholder} unchanged and move it freely for German word order. Never make a placeholder carry an article or a case ending that depends on its value – put a noun before it („die Notiz „{name}““, „in {vault}“, „mit {key}“). Plural keys need _one and _other; _one may use „1“ or „{n}“ as the English does; verbs agree with {n} („{n} Blöcke warten auf Prüfung“).
- Never translate: slash.* (typed commands), langName.*, the product name vemry (always lower case; rephrase so it does not open a sentence), file names and paths (VEMRY.md, .vemry/skills/), anything in backticks, field keys and the bytes written to disk (priority:: values, „every 2 weeks“, ISO dates, callout type names), Markdown syntax and URLs.
- English words with two senses get two German words: Review = „Prüfen“ for AI changes (ungeprüft, als geprüft markieren; never „ausstehend“ or „Entwurf“) but „Wiederholen“ for the flashcard projection (view.review, cmd.view-review.title); Plan = „Plan“ for the AI's plan, „Tarif“ for the subscription (account.row.plan); Done = „Erledigt“ for tasks and columns, „Fertig“ when an operation has finished (translate.complete); Reload = „Neu laden“; Later = „Später“ (agenda bucket and restart prompt alike).
- Words that must stay apart: Undo (⌘Z) = Rückgängig, Revert (an AI change) = zurücknehmen, Reject (a plan) = ablehnen – never swap them, and never „verwerfen“ or „zurücksetzen“ for revert. Collapse/Expand (panels, lists, thinking) = Einklappen/Ausklappen; Fold/Unfold (editor blocks) = Falten/Entfalten. Cancel (button) = Abbrechen, a cancelled task = abgebrochen, a cancelled subscription = gekündigt. Delete = löschen, Remove = entfernen. Clip = clippen/der Clip, never „ausschneiden“ (that is Cut) or „speichern“; quick capture = Schnellerfassung/erfassen.
- Projection names are fixed labels: Agenda, Board, Zeitleiste, Graph, Thesen, Belege, Sitzungswiedergabe, Leseansicht (Reader), Übersetzung (Translation), Wiederholen (Review). Projections in general are „Projektion“, never „Ansicht“, „Dashboard“ or „Modul“. In the two annotated term cells the parenthetical is a note, not UI text: the interface says „Leseansicht“, architecture prose „Leseprojektion“; „Parallelübersetzung“ names the concept, the projection's title is „Übersetzung“.
- Brevity: buttons, chips and menu items take the shortest idiomatic form – Ordner (not Verzeichnis), Tab (not Registerkarte), Seitenleiste (not Seitenbereich), no article in labels. Address the reader with du instead of naming a „Benutzer“ („attached by the user“ → „von dir angehängt“); no gender stars or colons.

## Words decided outside the glossary

Interface words the glossary does not cover, fixed while the catalog was written
so that every string uses the same one.

| English | German |
|---|---|
| subscription | Abo |
| entitlements | Freischaltungen |
| phone | Handy |
| fork (a session) / fork point | abzweigen / Abzweigpunkt |
| tool | Werkzeug |
| the Founder plan | Gründungsmitglied |
| thinking depth / thinking text | Denktiefe / Denkprozess |
| chat / conversation | Chat / Gespräch |
| follow-up (question) | Folgefrage |
| working note (the note the AI follows) | Arbeitsnotiz |
| action items | Aktionspunkte |
| method (a skill, as the picker calls it) | Methode |
| backlinks / outgoing links | Eingehende Links / Ausgehende Links |
| timeline (in the chat) | Verlauf |
| study ahead | Vorauslernen |
| write access | Schreibzugriff |
| system prompt | System-Prompt |
| paper / flashcards | Fachartikel / Lernkarten |
| the View menu (macOS) | Darstellung |
| callout type "note" | Anmerkung |
