// German is the language of the page itself. In English, every text that appears on the page
// is swapped as it appears, so the program logic keeps working with the German values.
(() => {
  const TEXTS = {
    // Navigation and frame
    "Bereiche": "Sections", "Tasten": "Buttons", "Makros": "Macros", "Sensor": "Sensor", "Einstellungen": "Settings",
    "Konfiguration": "Configuration", "Exportieren": "Export", "Importieren": "Import", "Zurücksetzen": "Reset", "Wirklich zurücksetzen?": "Really reset?",
    "Profil": "Profile", "Profil umbenennen": "Rename profile", "Eigener Name": "Custom name", "Name des Profils": "Profile name",
    "Verbunden": "Connected", "Nicht verbunden": "Not connected", "Maus inaktiv": "Mouse inactive", "Suche Maus": "Looking for mouse",
    // Buttons page
    "Du bearbeitest": "You are editing", "Aktuell:": "Current:", "Skizze der Maus": "Sketch of the mouse",
    "Rechte Taste": "Right button", "Linke Taste": "Left button", "Mausrad": "Mouse wheel", "Vordere Seitentaste": "Front side button", "Hintere Seitentaste": "Rear side button",
    "Maustasten": "Mouse buttons", "Linksklick": "Left click", "Rechtsklick": "Right click", "Mausrad-Klick": "Wheel click", "Vorwärts": "Forward", "Zurück": "Back",
    "Polling-Wechsel": "Polling switch", "Dauerfeuer": "Rapid fire", "Tastenkombination": "Key combination", "Deaktiviert": "Disabled", "Seitlich scrollen": "Side scroll",
    "Nach links scrollen": "Scroll left", "Nach rechts scrollen": "Scroll right", "DPI-Wechsel": "DPI switch", "Multimedia": "Multimedia", "Makro": "Macro",
    "Intervall (10–255)": "Interval (10–255)", "Wiederholungen (0–3)": "Repeats (0–3)", "Übernehmen": "Apply", "Hier klicken, Tasten drücken": "Click here, press keys",
    "Diese Funktion hat keine weiteren Optionen.": "This function has no further options.", "Unbekannte Tastenfunktion": "Unknown button function",
    "Noch keine Makros angelegt. Das geht im Bereich Makros.": "No macros yet. You can create them in the Macros section.",
    "Mindestens eine Taste muss Linksklick bleiben.": "At least one button must stay left click.",
    // Multimedia
    "Mediaplayer": "Media player", "Wiedergabe/Pause": "Play/Pause", "Nächster Titel": "Next track", "Vorheriger Titel": "Previous track", "Stopp": "Stop",
    "Stumm": "Mute", "Lauter": "Volume up", "Leiser": "Volume down", "E-Mail": "E-mail", "Rechner": "Calculator", "Arbeitsplatz": "My Computer",
    "Suche": "Search", "Startseite": "Home page", "Vorherige Seite": "Previous page", "Nächste Seite": "Next page", "Laden stoppen": "Stop loading", "Seite neu laden": "Reload page", "Favoriten": "Favorites",
    // Receiver light
    "Empfänger": "Receiver", "Empfänger-Licht": "Receiver light", "Lichteffekt": "Light effect", "Helligkeit": "Brightness", "Geschwindigkeit": "Speed", "Farbton": "Hue",
    "Sättigung und Helligkeit der Farbe": "Saturation and brightness of the colour",
    "Aus": "Off", "Feste Farbe": "Solid colour", "Einfarbig atmend": "Single colour breathing", "Regenbogen": "Rainbow", "Regenbogen atmend": "Rainbow breathing", "Neon": "Neon",
    // Macros
    "Makroliste": "Macro list", "Neues Makro": "New macro", "Löschen": "Delete", "Wirklich löschen?": "Really delete?", "Tastenfolge": "Key sequence", "Taste ändern": "Change key",
    "Aufnahme starten": "Start recording", "Aufnahme stoppen": "Stop recording", "Verzögerung": "Delay", "Automatisch aufzeichnen": "Record automatically", "Fester Wert (ms)": "Fixed value (ms)",
    "Feste Verzögerung in Millisekunden": "Fixed delay in milliseconds", "Wiederholen": "Repeat", "Bis diese Taste losgelassen wird": "Until this button is released",
    "Bis eine andere Taste gedrückt wird": "Until another button is pressed", "Anzahl": "Count", "Anzahl der Wiederholungen": "Number of repeats", "Befehl einfügen": "Insert command",
    "Verwerfen": "Discard", "Speichern": "Save", "Abbrechen": "Cancel", "Umbenennen": "Rename", "Makroname": "Macro name", "Name eingeben": "Enter a name", "Nr.": "No.", "Aktion": "Action", "Taste": "Key", "Danach (ms)": "Then (ms)",
    "↓ Drücken": "↓ Press", "↑ Loslassen": "↑ Release", "Taste drücken": "Press key", "Taste loslassen": "Release key", "Beispiel: Kopieren": "Example: Copy",
    "Aufnahme gespeichert.": "Recording saved.", "Änderungen zuerst speichern oder verwerfen.": "Save or discard your changes first.",
    "Der Name ist zu lang, Umlaute zählen doppelt.": "The name is too long, special characters count double.", "Diese Taste kann die Maus nicht senden.": "The mouse cannot send this key.",
    "Dieser Name ist schon vergeben.": "This name is already taken.", "Dieses Makro ist noch leer. Nimm zuerst Tasten auf.": "This macro is still empty. Record some keys first.",
    "Jetzt die gewünschte Taste drücken.": "Press the desired key now.", "Keine Taste aufgenommen, das Makro bleibt unverändert.": "No key recorded, the macro stays unchanged.",
    "Links ein Makro anlegen oder auswählen.": "Create or select a macro on the left.", "Noch keine Makros.": "No macros yet.",
    "Noch leer. Starte die Aufnahme und drücke die gewünschten Tasten.": "Still empty. Start recording and press the desired keys.",
    "Die gespeicherte Makroliste ist beschädigt.": "The saved macro list is damaged.", "Die gespeicherten Profilnamen sind beschädigt.": "The saved profile names are damaged.",
    // Keys
    "Strg links": "Left Ctrl", "Strg rechts": "Right Ctrl", "Umschalt links": "Left Shift", "Umschalt rechts": "Right Shift", "Win rechts": "Right Win", "Feststelltaste": "Caps Lock",
    "Leertaste": "Space", "Rücktaste": "Backspace", "Einfg": "Ins", "Entf": "Del", "Pos1": "Home", "Ende": "End", "Bild hoch": "Page Up", "Bild runter": "Page Down",
    "Pfeil hoch": "Arrow Up", "Pfeil runter": "Arrow Down", "Pfeil links": "Arrow Left", "Pfeil rechts": "Arrow Right", "Maustaste": "Mouse button",
    // Sensor
    "DPI-Stufen": "DPI stages", "Aktiv:": "Active:", "Als aktive Stufe setzen": "Set as active stage", "Stufe entfernen": "Remove stage", "+ Stufe hinzufügen": "+ Add stage",
    "Meldungen pro Sekunde": "Reports per second", "Debounce in Millisekunden": "Debounce in milliseconds", "Sensor-Funktionen": "Sensor functions",
    "Glättet Zittern im Sensorsignal, kann etwas Verzögerung bringen.": "Smooths jitter in the sensor signal, may add some delay.",
    "Begradigt Bewegungen zu geraden Linien.": "Straightens movements into lines.", "Stimmt die Sensordaten auf die Abfragen des PCs ab.": "Aligns the sensor data with the polling of the PC.",
    "Stärkere Funkverbindung auf Distanz, verkürzt die Akkulaufzeit.": "Stronger wireless link over distance, shortens battery life.",
    "Höchste Leistung": "Highest performance", "Der Sensor arbeitet im stärksten Modus, seine LED leuchtet hell.": "The sensor runs in its strongest mode, its LED shines brightly.",
    "Aktiv bis zum Sparmodus nach": "Active until power saving after", "Ruhemodus": "Sleep mode", "Maus schläft nach": "Mouse sleeps after",
    // Settings
    "Sprache": "Language", "Sprache wechseln": "Change language", "Start": "Startup", "Mit Windows starten": "Start with Windows", "H-Mouse Config startet unsichtbar im Tray.": "H-Mouse Config starts hidden in the tray.",
    "Tray-Symbol": "Tray icon", "Hintergrundfarbe nach Akkustand": "Background colour by battery level", "unter": "below", "darüber": "above",
    "Farbe bei niedrigem Akku": "Colour at low battery", "Farbe bei mittlerem Akku": "Colour at medium battery", "Farbe bei vollem Akku": "Colour at full battery",
    // Status
    "Importiere ...": "Importing ...", "Wechsle das Profil ...": "Switching profile ...",
    "Aktives Profil auf Standardwerte gesetzt (nur Beispielwerte).": "Active profile set to defaults (sample values only).",
    "Datei ist gültig. Im Entwurf wird nichts geschrieben.": "File is valid. Nothing is written in the draft.", "Im Entwurf gibt es keine Mausdaten zum Exportieren.": "The draft has no mouse data to export.",
    // Messages of the app part
    "Die Mausverbindung läuft nicht.": "The mouse connection is not running.", "Die Mausverbindung wurde unterbrochen.": "The mouse connection was interrupted.",
    "Ungültiges Makro-Ereignis.": "Invalid macro event.", "Ungültige Taste in der Kombination.": "Invalid key in the combination.",
    "Keine gültige Konfigurationsdatei.": "Not a valid configuration file.", "Die Datei passt nicht zu dieser Maus.": "The file does not match this mouse.",
    "Die Datei enthält keine DPI-Stufe.": "The file contains no DPI stage.", "In der Datei hat keine Taste Linksklick.": "No button in the file has left click.",
    "Maus nicht verbunden.": "Mouse not connected.", "Die Maus hat das Schreiben nicht bestätigt.": "The mouse did not confirm the write.",
    "Der Wert konnte nicht zurückgelesen werden.": "The value could not be read back.", "Der zurückgelesene Wert weicht ab.": "The value read back differs.",
    "Ungültige DPI-Stufen.": "Invalid DPI stages.", "Ungültige Lichtwerte.": "Invalid light values.", "Das Empfänger-Licht wurde noch nicht gelesen.": "The receiver light has not been read yet.",
    "Der Empfänger hat das Schreiben nicht bestätigt.": "The receiver did not confirm the write.", "Unbekannte Taste.": "Unknown button.",
    "Die Maus hat den Profilwechsel nicht bestätigt.": "The mouse did not confirm the profile switch.", "Die Maus hat das Profil nicht gewechselt.": "The mouse did not switch the profile.",
    "Das Profil konnte nicht gelesen werden. Bitte die Maus kurz bewegen.": "The profile could not be read. Please move the mouse briefly.",
    "Die Maus ist inaktiv. Bitte kurz bewegen und erneut versuchen.": "The mouse is inactive. Please move it briefly and try again.",
    "Noch keine Mausdaten gelesen.": "No mouse data read yet.", "Die untere Schwelle muss kleiner sein als die obere.": "The lower threshold must be below the upper one.", "Ungültige Farbe.": "Invalid colour."
  };

  const PATTERNS = [
    [/^Profil (\d+)( – .*)?$/, "Profile $1$2"],
    [/^Akku (\d+) %( \(lädt\))?(?: · Maus (\d+) %(, \d+ mV)?)?$/, (_, level, charging, reported, voltage) => `Battery ${level} %${charging ? " (charging)" : ""}${reported ? ` · Mouse ${reported} %${voltage || ""}` : ""}`],
    [/^(\d+) Min\.$/, "$1 min"],
    [/^(\d+),(\d+) mm$/, "$1.$2 mm"],
    [/^Stufe (\d+)$/, "Stage $1"],
    [/^Stufe (\d+) aktiv$/, "Stage $1 active"],
    [/^Stufe (\d+) entfernen$/, "Remove stage $1"],
    [/^DPI von Stufe (\d+)$/, "DPI of stage $1"],
    [/^DPI-Wert von Stufe (\d+)$/, "DPI value of stage $1"],
    [/^Maximal (\d+) Stufen$/, "Maximum $1 stages"],
    [/^Maximal (\d+) Ereignisse pro Makro\.$/, "Maximum $1 events per macro."],
    [/^(\d+) Ereignisse erreicht, Aufnahme gestoppt\.$/, "$1 events reached, recording stopped."],
    [/^Aufnahme läuft: (\d+) von (\d+) Ereignissen\.$/, "Recording: $1 of $2 events."],
    [/^Verzögerung nach Ereignis (\d+)$/, "Delay after event $1"],
    [/^Tastenkombination \(max\. (\d+)\)$/, "Key combination (max. $1)"],
    [/^Tastenfolge: (.*)$/, "Key sequence: $1"],
    [/^Gespeichert: (.*)$/, "Saved: $1"],
    [/^Importiert: (.*)$/, "Imported: $1"],
    [/^Farbe (#[0-9a-f]{6})$/, "Colour $1"],
    [/^Maustaste (.*)$/, "Mouse button $1"],
    [/^Modifikator (.*)$/, "Modifier $1"],
    [/^Taste (.*)$/, "Key $1"],
    [/^Unbekannt \((.*)\)$/, "Unknown ($1)"],
    [/^(.+) umbenennen$/, "Rename $1"],
    [/^(.+) entfernen$/, "Remove $1"],
    [/^Sättigung (\d+) %, Helligkeit (\d+) %$/, "Saturation $1 %, brightness $2 %"],
    [/^Ungültiger Wert (.*)\.$/, "Invalid value $1."],
    [/^Ungültiger DPI-Wert (.*)\.$/, "Invalid DPI value $1."],
    [/^Ungültige Tastenfunktion (.*)\.$/, "Invalid button function $1."],
    [/^Ungültige Wiederholung (.*)\.$/, "Invalid repeat $1."],
    [/^Der Makroname darf 1 bis (\d+) Bytes lang sein\.$/, "The macro name may be 1 to $1 bytes long."],
    [/^Eine Kombination braucht 1 bis (\d+) Tasten\.$/, "A combination needs 1 to $1 keys."],
    [/^Unbekanntes Profil (.*)\.$/, "Unknown profile $1."],
    [/^Polling Rate (\d+) Hz wird hier nicht unterstützt\.$/, "Polling rate $1 Hz is not supported here."],
    [/^Unbekannte Multimedia-Funktion (.*)\.$/, "Unknown multimedia function $1."]
  ];

  function english(text) {
    const core = text.trim();
    if (!core) return text;
    let result = TEXTS[core];
    if (result === undefined) {
      const pattern = PATTERNS.find(([test]) => test.test(core));
      if (!pattern) return text;
      result = core.replace(pattern[0], pattern[1]);
    }
    return text.replace(core, () => result);
  }

  const STORAGE_KEY = "language";
  const ATTRIBUTES = ["title", "aria-label", "placeholder", "alt", "aria-valuetext"];
  const originals = new WeakMap();
  let language = "de";
  try { language = localStorage.getItem(STORAGE_KEY) || (navigator.language.startsWith("de") ? "de" : "en"); } catch (error) { /* keeps German */ }

  // Names chosen by the user stay as they are.
  const isExcluded = element => !element || element.closest("script, style, .macro-row");

  function swapText(node) {
    if (language !== "en" || isExcluded(node.parentElement)) return;
    const known = originals.get(node);
    if (known && known.en === node.data) return;
    const en = english(node.data);
    if (en === node.data) return;
    originals.set(node, { de: node.data, en });
    node.data = en;
  }

  function swapAttribute(element, name) {
    if (language !== "en" || !element.hasAttribute(name) || isExcluded(element)) return;
    const value = element.getAttribute(name);
    const store = originals.get(element) || {};
    if (store[name] && store[name].en === value) return;
    const en = english(value);
    if (en === value) return;
    store[name] = { de: value, en };
    originals.set(element, store);
    element.setAttribute(name, en);
  }

  function swapTree(root) {
    if (root.nodeType === Node.TEXT_NODE) { swapText(root); return; }
    if (root.nodeType !== Node.ELEMENT_NODE) return;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
    for (let node = root; node; node = walker.nextNode()) {
      if (node.nodeType === Node.TEXT_NODE) swapText(node);
      else ATTRIBUTES.forEach(name => swapAttribute(node, name));
    }
  }

  function restoreTree(root) {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
    for (let node = root; node; node = walker.nextNode()) {
      const known = originals.get(node);
      if (!known) continue;
      if (node.nodeType === Node.TEXT_NODE) {
        if (node.data === known.en) node.data = known.de;
      } else {
        ATTRIBUTES.forEach(name => {
          if (known[name] && node.getAttribute(name) === known[name].en) node.setAttribute(name, known[name].de);
        });
      }
    }
  }

  new MutationObserver(records => {
    for (const record of records) {
      if (record.type === "characterData") swapText(record.target);
      else if (record.type === "attributes") swapAttribute(record.target, record.attributeName);
      else record.addedNodes.forEach(swapTree);
    }
  }).observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRIBUTES });

  window.appLanguage = {
    get: () => language,
    set(next) {
      language = next;
      document.documentElement.lang = next;
      try { localStorage.setItem(STORAGE_KEY, next); } catch (error) { /* only a convenience */ }
      if (next === "en") swapTree(document.body);
      else restoreTree(document.body);
    }
  };
  document.documentElement.lang = language;
  // This script runs after the static page, which the observer does not see as new.
  swapTree(document.body);
})();
