# Slotboard – Anleitung

Zehn Fächer für deine Zwischenablage.

| | Mac | Windows & Linux |
|---|---|---|
| **Ablegen** | ⌘C drücken, ⌘ halten, Ziffer 1–0 tippen | Strg+C drücken, Strg halten, Ziffer tippen |
| **Einfügen** | ⌘V drücken, ⌘ halten, Ziffer 1–0 tippen | Strg+V drücken, Strg halten, Ziffer tippen |

Ohne Ziffer funktionieren Kopieren und Einfügen ganz normal.

---

## Installation auf dem Mac

1. Auf der [Release-Seite](https://github.com/markuskoppeberlin-a11y/slotboard/releases) die Datei **`Slotboard_…_universal.dmg`** laden. Sie läuft auf allen Macs ab macOS 11, mit Apple-Chip oder Intel.
2. Die Datei öffnen und **Slotboard in den Programme-Ordner ziehen**.
3. Slotboard im Programme-Ordner per Doppelklick öffnen.
4. macOS meldet beim ersten Mal, dass die App nicht überprüft werden kann. Das liegt daran, dass Slotboard (noch) nicht bei Apple registriert ist, und kommt nur einmal:
   - Auf **Fertig** klicken.
   - **Systemeinstellungen → Datenschutz & Sicherheit** öffnen und nach unten scrollen.
   - Bei „Slotboard wurde blockiert …“ auf **Dennoch öffnen** klicken und mit Passwort oder Touch ID bestätigen.
5. Jetzt öffnet sich die Einrichtung. Sie führt dich durch den Rest:
   - **Zugriff erlauben** klicken.
   - Im Dialog **Systemeinstellungen öffnen** wählen und **Slotboard einschalten**.
   - Zurück zu Slotboard, es geht automatisch weiter.
   - Kurz ausprobieren, fertig.

Slotboard sitzt danach oben in der **Menüleiste** (Symbol mit kleinen Kästchen). Ein Klick darauf zeigt deine Fächer.

### Neue Version installieren (Mac)

1. Slotboard über das Menüleisten-Symbol → **Slotboard beenden** schließen.
2. Die neue `.dmg` öffnen und Slotboard wieder in den Programme-Ordner ziehen (**Ersetzen** wählen).
3. Öffnen. Die Gatekeeper-Meldung von oben kommt einmal wieder.
4. Weil jede Testversion für macOS eine „neue“ App ist, muss der Zugriff unter Bedienungshilfen neu erteilt werden. Die Einrichtung räumt den alten Eintrag selbst auf, du musst Slotboard nur wieder einschalten.

Sobald Slotboard bei Apple registriert ist, entfallen die Schritte 3 und 4.

---

## Installation unter Windows

1. Auf der [Release-Seite](https://github.com/markuskoppeberlin-a11y/slotboard/releases) **`Slotboard_…_x64-setup.exe`** laden und starten.
2. Falls Windows „Der Computer wurde durch Windows geschützt“ meldet: **Weitere Informationen → Trotzdem ausführen**.
3. Slotboard startet und zeigt die kurze Einrichtung. Unter Windows ist keine extra Freigabe nötig.

Slotboard sitzt danach im **Infobereich der Taskleiste** (unten rechts, ggf. hinter dem Pfeil ^).

Hinweis: Strg+Alt wirkt unter Windows wie AltGr. Slotboard nutzt nur Strg + C/V + Ziffer, AltGr-Zeichen wie { [ ] } funktionieren weiter normal.

---

## Installation unter Linux

- **`.deb`** (Ubuntu, Debian, Mint): per Doppelklick installieren oder `sudo apt install ./slotboard_…_amd64.deb`.
- **`.AppImage`**: ausführbar machen (`chmod +x`) und starten.

Slotboard braucht eine **X11-Sitzung**. Unter Wayland erlauben die Systeme keine globalen Tastenkürzel. Bei Ubuntu wählst du beim Anmelden unten rechts „Ubuntu on Xorg“.

---

## Gut zu wissen

- **Fächer** halten alles, was auch die normale Zwischenablage kann: Text mit und ohne Formatierung, Bilder, Dateien und Inhalte aus Apps wie Figma.
- **Normale Zwischenablage bleibt erhalten.** Nach ⌘V + Ziffer liegt in der normalen Zwischenablage wieder das, was vorher drin war.
- **Fächer sind flüchtig.** Beim Beenden von Slotboard oder Ausschalten des Computers werden sie geleert.
- **Passwörter** aus Passwortmanagern werden nicht in Fächer übernommen.
- **Pausieren:** Menüleisten-Symbol → *Pausieren*. Dann ist Slotboard komplett inaktiv.
- **Beim Anmelden starten** lässt sich im Menü und im Hilfe-Fenster ein- und ausschalten.
- **Hilfe-Fenster:** Slotboard noch einmal öffnen (z. B. im Programme-Ordner) oder im Menü *Hilfe & Einrichtung…* wählen.

## Etwas klappt nicht?

| Problem | Lösung |
|---|---|
| ⌘V + Ziffer fügt nichts ein | Ist das Fach belegt? Im Menüleisten-Symbol nachsehen. |
| Gar nichts reagiert (Mac) | Menü → *Einrichtung abschließen* bzw. *Hilfe & Einrichtung* und den Zugriff erneut erlauben. |
| Slotboard steht unter Bedienungshilfen, ist eingeschaltet, geht aber nicht | Eintrag mit „–“ entfernen, dann in Slotboard erneut **Zugriff erlauben**. |
| In einem Passwortfeld geht es nicht | Absicht: macOS schützt Passwortfelder vor allen Tastatur-Tools. |
