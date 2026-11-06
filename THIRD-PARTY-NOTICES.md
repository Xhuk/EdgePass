# Avisos de terceros

edgePass es MIT. Este archivo dice qué material ajeno entra al ejecutable y bajo qué licencia.

## Lista de palabras Diceware

`src-tauri/data/diceware-en.txt` contiene 7.411 palabras tomadas de la lista inglesa
de Diceware, compilada por Arnold G. Reinhold.

```
Diceware word list
Copyright (c) Arnold G. Reinhold
Licensed under the Creative Commons Attribution 4.0 license (CC-BY 4.0).
https://creativecommons.org/licenses/by/4.0/
Source: https://theworld.com/~reinhold/diceware.wordlist.asc
```

**Qué se modificó.** La lista original tiene 7.776 entradas. Se quitaron 365 que
contenían apóstrofes, símbolos o cifras (`a's`, `can't`, `a&p`, `@`, `1234`), y se
dejaron solo palabras de letras `a-z`. Eso permite que la variante "compatible"
prometa únicamente caracteres alfanuméricos. La selección y el orden se conservaron.

**Qué no se modificó.** Ninguna palabra se tradujo ni se normalizó. El promedio de
longitud quedó en 4,29 letras.

## Código propio

El generador (`src-tauri/src/passphrase.rs`) y el azar (`src-tauri/src/rand.rs`) están
escritos para este proyecto. No contienen código de ninguna implementación de Diceware
de terceros.

En particular, **no** se usa la implementación de Diceware en Python de Uli Fouquet ni
ninguna otra bajo GPL. Un fragmento GPL dentro de edgePass obligaría a publicar la app
completa bajo GPL, y este proyecto es MIT.

El método de Diceware en sí es una técnica publicada, no código: elegir palabras de una
lista con azar uniforme. Eso no se licencia.

## Atribución en la interfaz

La licencia CC-BY se cumple con este archivo. No hace falta mostrarlo en la ventana.
