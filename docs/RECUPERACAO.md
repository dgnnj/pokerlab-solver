# Recuperação manual

## Instalação interrompida ou app não abre após atualizar
1. Baixe o instalador da versão desejada na página de Releases (arquivo `PokerLab_X.Y.Z_x64-setup.exe`) e confira o SHA-256 publicado.
2. Execute-o normalmente. Ele substitui os binários em `%LOCALAPPDATA%\PokerLab` e **não** toca nos dados.
3. Seus dados ficam em `%LOCALAPPDATA%\dev.pokerlab.trainer\` (banco `pokerlab.sqlite3`, pacotes, backups).

## Banco incompatível ("banco em versão N mais nova que a suportada")
Acontece ao voltar para uma versão anterior depois de uma migração. Opções:
- Reinstale a versão mais nova; ou
- Restaure um backup: feche o app, renomeie `pokerlab.sqlite3`, copie o arquivo desejado de `backups\` (ex.: `pokerlab-pre-migracao-v1-<data>.sqlite3`) para `pokerlab.sqlite3` e reabra.

## Pacote de exercícios corrompido
O app verifica SHA-256 e tamanho a cada abertura e reinstala o pacote a partir do instalador. Se persistir, apague `%LOCALAPPDATA%\dev.pokerlab.trainer\packs\` e reabra.

## Log de falha de inicialização
`%LOCALAPPDATA%\dev.pokerlab.trainer\startup-error.log`. Não contém histórico de mãos.
