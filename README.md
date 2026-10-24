# ak820ctl

Controle do **AJAZZ AK820 Pro** com o firmware original, para **Linux** (inclusive Bazzite) e **Windows**, pelo **cabo USB** ou pelo **dongle 2.4G**. Não grava firmware.

São dois programas:
- **`ak820gui`:** interface gráfica com as abas Iluminação, Teclas (cor por tecla e troca de função, num teclado desenhado) e Tela (imagem/GIF com prévia e relógio). Conecta sozinha e mostra o firmware.
- **`ak820ctl`:** linha de comando, boa para scripts e atalhos.

Os dois podem ficar abertos junto com o programa oficial: as respostas que o programa oficial provoca são ignoradas.

## O que funciona

| Comando | Cabo | Dongle 2.4G |
|---|---|---|
| `mode <efeito> [-c RRGGBB] [-b 0-5] [-s 0-5] [-d 0-1]` | ✅ | ✅ |
| `modes`: lista dos 20 efeitos | ✅ | ✅ |
| `ping`: confere se o teclado responde (só leitura) | ✅ | ✅ |
| `list`: interfaces encontradas | ✅ | ✅ |
| `keys --all 000020 w=ff0000 a=ff0000`: cor por tecla | ✅ | ❌ (nem o programa oficial oferece em 2.4G) |
| `time`: acerta o relógio da tela | ✅ | ✅ |
| `info`: modelo e versão do firmware | ainda não | ✅ |
| `remap <tecla> <função>`: troca a função de uma tecla (salva no teclado) | ainda não* | ✅ |
| `screen arquivo [--pad]`: imagem, GIF ou vídeo curto na tela de 128×128 | ✅ | ❌ (o firmware original não aceita) |
| `hyperhdr --host IP`: acompanha as cores do HyperHDR | ✅ tempo real | ✅ com limite |

\* Falta capturar o comando pelo cabo. A troca feita pelo dongle fica salva no teclado e vale também no cabo.

Funções do `remap`:
- `default`;
- uma tecla, com `ctrl+`, `shift+`, `alt+` ou `win+` opcionais (ex.: `ctrl+shift+t`);
- `key:0xNN` (uso HID);
- `media:volup|voldown|mute|play|next|prev|stop|calc|mail|browser|mycomputer`.

A tecla Fn não pode ser trocada.

Testado em 2026-10-22 num AK820 Pro `0c45:800a` (firmware 1.10) com dongle `0c45:fdfd` (1.38), no Windows 11:
- pelo dongle: luz, relógio, info e troca de tecla (F5 → Volume + e volta);
- pelo cabo: luz, cor por tecla, relógio e imagem (o envio leva ~2 s).

A versão de Linux ainda não foi testada em hardware.

A conexão é escolhida sozinha: cabo se a chave estiver em USB, senão dongle. Para forçar: `--via wired` ou `--via dongle`.

## Vídeos curtos na tela

```
ak820ctl screen clipe.mp4                          # vídeo direto para a tela (cabo)
ak820ctl screen clipe.mp4 --fps 12 --start 3 --duration 8
ak820ctl screen clipe.mp4 --save-gif clipe.gif --no-upload   # só converte para GIF 128×128
```

Aceita mp4, m4v, mkv, webm, mov e avi. O vídeo é reduzido para 128×128, recortado no centro ou com `--pad` para bordas. A tela guarda até **140 quadros**: a 10 quadros/s cabem 14 s, a 16 quadros/s cerca de 9 s. A tela não passa de ~16 quadros/s.

Na interface, a aba **Tela** aceita vídeos, com quadros/s, início e duração ajustáveis, prévia animada e o botão **Salvar como GIF…**.

Precisa do **ffmpeg** instalado:
- **Windows:** `winget install Gyan.FFmpeg`;
- **Linux:** o pacote `ffmpeg` da distro, que no Bazzite já vem instalado.

Para usar outro executável, indique o caminho na variável `AK820_FFMPEG`.

## Sincronizar com o HyperHDR

```
ak820ctl hyperhdr --host 192.168.0.10        # IP do HyperHDR (127.0.0.1 se for no mesmo PC)
```

O programa lê as cores que o HyperHDR já está mandando para as fitas, pela API JSON dele (comando `ledcolors`/`ledstream-start`). Não precisa mudar nada no HyperHDR além de deixar o **servidor JSON** ligado (porta padrão 19444). Ele faz a média das cores, ignorando bordas pretas, e aplica no teclado:

- **pelo cabo:** em tempo real, ~15 vezes por segundo, sem salvar na memória do teclado;
- **pelo dongle:** cor do teclado inteiro. O único comando de cor pelo rádio provavelmente salva na memória, então a cor só troca quando muda de verdade (`--threshold`, padrão 24) e no máximo a cada `--min-gap` segundos (padrão 2).

Se a conexão cair, ele tenta de novo a cada 5 s.

## Ganchos (comandos seus em eventos do teclado)

Crie `hooks.json` em `~/.config/ak820ctl/` (Linux) ou `%APPDATA%\ak820ctl\` (Windows):

```json
{
  "on_connect": "notify-send 'AK820' \"conectado: $AK820_DETAIL\"",
  "on_disconnect": "/home/voce/bin/ak820-caiu.sh",
  "on_error": ""
}
```

| Evento | Quando |
|---|---|
| `on_connect` | a interface conecta no teclado |
| `on_disconnect` | a conexão com o teclado ou com o dongle cai |
| `on_error` | um envio falha, ou a sincronização com o HyperHDR cai |

O comando roda em segundo plano pelo shell do sistema, com as variáveis `AK820_EVENT` e `AK820_DETAIL`. É o lugar para plugar ações próprias, como um script que reinicia o dongle.

## Exemplos

```
ak820ctl mode static -c 00ff00          # verde fixo
ak820ctl mode breath -c ff0080 -s 2     # respiração rosa
ak820ctl mode spectrum                  # sem -c = arco-íris
ak820ctl mode off
ak820ctl --via wired screen gato.gif
```

## Instalação

**Windows:** abra o `ak820gui.exe` (ou use o `ak820ctl.exe` num terminal). Não precisa instalar nem de driver.

### Bazzite, passo a passo

1. Baixe o código:
   ```
   git clone https://github.com/DarkTiengo/ak820ctl.git && cd ak820ctl
   ```
   Para atualizar depois: `git pull` e repetir os passos 2 e 3.
2. Compile. Isso cria um container Fedora com distrobox; a primeira vez demora alguns minutos.
   ```
   bash linux/build-bazzite.sh
   ```
3. Instale. Pede a senha do `sudo` só para a regra udev.
   ```
   bash dist-linux/install.sh --hyperhdr
   ```
   Sem `--hyperhdr`, só o relógio automático fica ligado.
4. Se precisar, ajuste `~/.config/ak820ctl/ak820.env` (endereço do HyperHDR, `AK820_VIA`, intervalo) e reinicie o serviço:
   ```
   systemctl --user restart ak820-hyperhdr.service
   ```

**Serviços instalados (systemd do usuário, rodam só com você logado):**

| Unidade | O que faz |
|---|---|
| `ak820-time.timer` | acerta o relógio da tela 30 s depois de entrar e a cada hora |
| `ak820-hyperhdr.service` | mantém a luz do teclado acompanhando o HyperHDR; reinicia sozinho se cair |

Acompanhar o que acontece:

```
systemctl --user status ak820-hyperhdr.service
journalctl --user -u ak820-hyperhdr.service -f
```

Para desinstalar, rode `bash dist-linux/uninstall.sh`; com `--all`, apaga também a configuração.

**Outras distros Linux (com os binários prontos):** na pasta com os arquivos, rode:

```
./install.sh
```

O script:
- copia `ak820ctl` e `ak820gui` para `~/.local/bin`;
- cria o atalho "AK820 Pro" no menu;
- instala a regra udev em `/etc/udev/rules.d/70-ak820.rules`, que libera o acesso ao teclado e ao dongle sem `sudo`.

Não usa `rpm-ostree`. A interface roda em Wayland e X11.

## Compilar

Precisa de Rust estável.

No Linux também precisa de:
- **Fedora:** `systemd-devel libxkbcommon-devel wayland-devel mesa-libGL-devel`;
- **Debian/Ubuntu:** os pacotes listados em `.github/workflows/build.yml`.

**No Bazzite**, o script faz tudo: cria um container Fedora com distrobox, instala as dependências dentro dele, compila e põe os binários em `dist-linux/`.

```
bash linux/build-bazzite.sh
bash dist-linux/install.sh
```

No Windows, se a pasta tiver um caminho muito longo, use `CARGO_TARGET_DIR` com um caminho curto para o linker não falhar.

```
cargo test
cargo build --release
```

O workflow `.github/workflows/build.yml` gera os executáveis de Windows e Linux a cada push.

## Protocolo

- **Cabo:** feature reports de 64 bytes na interface `0xFF13`, em sessões `04 18 … 04 02 … 04 F0`. A tela usa relatórios de 4096 bytes na `0xFF68`. Fonte: [hcristosm/ak820-linux](https://github.com/hcristosm/ak820-linux) (MIT), a partir do programa oficial v1.0.0.5.
- **Dongle `0C45:FDFD`:** relatórios de 32 bytes na interface `0xFF60`, no formato `cmd, tamanho, arg, dados…, soma`. A soma ocupa o byte 31 e é a soma dos bytes 0–30. O teclado responde em ~12 ms. Capturado com USBPcap em 2026-10-22:

  | Quadro | Função |
  |---|---|
  | `02 00 00` | informações; a resposta traz VID, PID e versão nos bytes 7–12 |
  | `03 04 <tecla>` + `02 mod uso 00` / `03 uso_lo uso_hi 00` | função da tecla (comum / mídia) |
  | `05 10 00` + 16 bytes | luz (mesmos dados do cabo) |
  | `0C 10 00` + 16 bytes | relógio (mesmos dados do cabo, com `AA 55` nos bytes 14–15) |
  | `20 01` | consulta de estado |
  | `0B 1C 00` + `04 00 64` + até 18 níveis de 0 a 6 | modo de música em tempo real (~33 por segundo, não salva): níveis de um equalizador; as cores são escolhidas pelo teclado |
  | `09 1c <0..0f>` + 28 bytes, depois `09 xx 10` … `AA 55` | tabela de macros inteira em 17 pedaços (476 bytes). Ainda não decodificada: começa com os deslocamentos de cada macro (ex.: `90 01`, `a8 01` = 400, 424); cada macro tem ~24 bytes com eventos `tecla, b0` (aperta) / `tecla, 30` (solta) e um campo `0a` (atraso?) |

## Licença

MIT.
