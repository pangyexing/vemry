# Spanish (`es`) translation conventions

The rules `es.json` is written to. Concept words come from
[docs/GLOSSARY.md](../../docs/GLOSSARY.md); these settle everything else two
translators would otherwise do differently. A new string follows them, and a
changed rule changes the catalog with it.

## Style

- Register: tú throughout (imperative tú for instructions: “Abre una bóveda primero”), never usted, vosotros or voseo. Neutral vocabulary: archivo (not fichero), hacer clic (not pinchar/cliquear), teléfono (not móvil/celular), “tu equipo” or a rephrasing instead of computadora/ordenador, atajo for a keyboard shortcut. No “por favor”.
- Buttons, menu items, command titles and chips that trigger something use the infinitive with no article: “Guardar”, “Abrir bóveda…”, “Cerrar pestaña”, “Revertir cambio de la IA”. Where one English word is both noun and verb (Clip, Review, Edit), the infinitive goes on the control that acts and the noun on a group, view, column or origin label (“Recortar” on the button, “Recorte” as a group or provenance origin). Menu bar: Archivo, Editar, Ver, Ventana, Ayuda.
- Descriptions and hints (cmd.*.desc, *Hint, placeholders): present indicative when saying what the control does (“Abre la nota en el bloque citado”), tú imperative when telling the user what to do (“Elige una carpeta”, “Inicia sesión en Ajustes › Cuenta”). For regular verbs the two forms are identical; do not switch to infinitive in running sentences.
- Sentence case for every label, heading, tab, projection and menu item, including the ones English writes in Title Case (“Buscar actualizaciones…”, “Licencias de código abierto”, “Reproducción de sesión”). Weekdays and months lowercase. “vemry” is always lowercase, even at the start of a sentence; AI is “IA” (la IA); Markdown, PDF, URL, Pro stay as is.
- Punctuation: always open ¿ and ¡ (avoid exclamations in the interface altogether). Quotation marks are “ ” with ‘ ’ inside them, never « » and never straight quotes in copy; a period or comma goes after the closing quote. Keep English separators as they are: › in paths (“Ajustes › Zonas privadas”), · between items, → for moves.
- The English spaced em dash “ — ” is never copied. Use a colon when the second part explains or names (“Zona privada: invisible para la IA”), a semicolon or a new sentence for an independent clause, parentheses or commas for an aside. If a true inciso needs a raya, write it the Spanish way, closed up: “texto —inciso— texto”.
- Ellipsis is the single character “…”, no space before it, kept exactly where English has it: items that open a dialog or picker (“Cambiar de bóveda…”), placeholders, progress (“Cargando…”). Never three periods.
- Genders, so articles and adjectives agree: la bóveda, la nota, la carpeta, la sección, la proyección, la habilidad, la zona privada, la memoria de la bóveda, la pestaña, la barra lateral, la paleta de comandos, la cuenta, la tarea, la tarjeta, la sesión, la afirmación, la evidencia, la referencia, la cita, la incrustación, la procedencia, la IA; el bloque, el campo, el paso, el chip, el panel, el esquema, el script, el registro de auditoría, el recorte, el marcador, el paquete de créditos, el cupo, el token, el modelo, el diff, el enlace, el adjunto, el encabezado. “asa” is feminine but takes el in the singular: el asa del bloque, las asas. vemry as a subject takes the masculine (“vemry alojado está en pausa”).
- Tasks are feminine (la tarea), so statuses, columns and chips agree: Vencidas, Programadas, Sin programar, completada el, cancelada el, creada el, bloqueada. Task chips are short phrases: “se repite {rule}”, “estimación {v}”, “ID {id}”, “vence {date}”, “se quita al completarse”. Priorities as chips: urgente, importante, más adelante, algún día.
- Placeholders stay exactly as in English. Never make an article or adjective agree with a placeholder whose gender is unknown: name the noun first (“la nota {name}”, “el archivo {name}”) or put the placeholder after a colon (“Recortado en: {target}”). Plurals: _one for exactly 1 (“{n} nota”, keep {n}), _other for the rest, _many only for large round numbers (millions, compact forms) and written identically to _other; participles agree in number with {n} (“{n} bloques movidos”).
- Errors: impersonal “No se pudo + infinitive” (“No se pudo guardar el adjunto {name}”), then what to do in tú imperative after a colon or semicolon. No “Error:” prefix, no apologies, no blame on the user. Success: a past participle agreeing with what it refers to (“Copiado”, “Página añadida a marcadores”). Confirmations ask with ¿…? and the buttons name the action (“¿Eliminar la sesión?” → “Eliminar” / “Cancelar”), never Sí/No.
- The AI is a tool: “la IA” or “el modelo” as subject of mechanical verbs only (lee, escribe, propone, redacta, cita, busca, traduce); never piensa, cree, quiere, sabe, entiende, decide, recuerda or “le gustaría”, no first person from the AI, never “asistente”. The model’s thinking setting is “razonamiento” (“Profundidad de razonamiento”, status “Razonando…”). Permission dialog: “Permitir”, “Denegar”, “Editar y aplicar”; plans: “Aprobar” / “Rechazar”.
- Keys (key.*): Ctrl, Mayús, Alt, Retroceso, Win; the macOS symbols ⌘ ⌥ ⇧ ⌃ stay; Enter, Esc and Tab stay as written in English inside sentences (“Enter: esta sección · Esc: volver”). An API key is a “clave” (clave de API, tu propia clave); a keyboard key is a “tecla”.
- Verb pairs that must not blur: Eliminar = destroys data (file, session, note); Quitar = takes off a list or a note without destroying it (“Quitar campo”, “Quitar marcador”); Borrar = clears (“Borrar filtro”, “Borrar historial”); Deshacer = only the editor’s ⌘Z; Revertir = an AI change; Cancelar = abandon a dialog or operation. Repaso/repasar = flashcard review (the Repaso view); revisar/revisado/sin revisar = checking AI changes. “Más adelante” = later dates or priority; “Más tarde” = a button that postpones.
- What is typed or written to disk stays in English: slash.* commands, frontmatter keys, `key:: value` keys, callout type names in the syntax, folder names (clips/, research/, journal/), VEMRY.md, .vemry/, ISO dates, `every 2 weeks`, `3d2h`. Technical words kept as loanwords: diff (los diffs), frontmatter, token(s), script. Credits are always “créditos”, never money.
- The Spanish equivalents of every GLOSSARY.md “Do not use” word are excluded too: not repositorio / espacio de trabajo / biblioteca for bóveda; not propiedad / atributo / metadatos for campo; not insignia / píldora / etiqueta for chip (etiqueta is #tag); not complemento / extensión for habilidad; not historial / actividad for registro de auditoría; not pendiente / borrador for sin revisar; not rechazar / deshacer for revertir; not contraer for plegar (contraer / expandir are only for panels and lists); not fecha límite for fecha de vencimiento; not capturar / guardar for recortar; not favorito / estrella for marcador; not panel for barra lateral; not tabla de contenido for esquema.
- Concision: prefer the shorter of two equally good words (Renombrar over Cambiar nombre, Ajustes over Configuración); “New X” is “Nuevo/Nueva X” agreeing with X (“Nueva bóveda”, “Nueva habilidad”); drop articles in labels and chips where English has none; navigation buttons are Atrás / Adelante, while “back to X” in a hint is “volver a X”. Relative time: hoy, mañana, ayer, hace un momento; dates and numbers come from Intl, never hand-formatted.

## Words decided outside the glossary

Interface words the glossary does not cover, fixed while the catalog was written
so that every string uses the same one.

| English | Spanish |
|---|---|
| follow (the note the AI works in) | seguir ("Hacer que la IA siga…") |
| undo (an AI turn's writes) / undo (editor, file tree) | Revertir / Deshacer |
| remove / delete | Quitar / Eliminar |
| agent | el agente |
| endpoint | endpoint |
| system prompt | prompt de sistema |
| pair (a phone) | vincular |
| jot (a convention with #) / audit log | anotar / registro |
| attachment bucket | carpeta |
| done (a task) / the Done column | completada / Completadas |
| action items | Puntos de acción |
| a count in a key without plural forms | "label: {n}" |
