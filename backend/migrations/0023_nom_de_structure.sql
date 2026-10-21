-- Nom d'une structure : 100 caractères au plus, sans retour à la ligne ni
-- caractère de contrôle (décision du porteur, 2026-10-21). Raison : le nom
-- est repris dans le SMS d'invitation (étape 3.10) ; avec 100 caractères de
-- l'alphabet GSM 7 bits, le SMS tient en 2 parties au plus (3GPP TS 23.040
-- §9.2.3.24.1 : 153 caractères par partie). Le code fait les mêmes
-- contrôles avant l'écriture, pour renvoyer un message clair
-- (`structure.rs`).
--
-- Longueur en caractères (points de code), comme `char::count` côté Rust :
-- doc PostgreSQL 18, fonctions de chaînes, `char_length` « Returns number
-- of characters in the string ».
--
-- Caractères refusés, listés un par un en échappements longs (règle de la
-- section 13 du CLAUDE.md, même méthode que la migration 0012) :
-- - les 65 caractères de contrôle de la catégorie Unicode Cc, U+0000 à
--   U+001F et U+007F à U+009F, la définition de `char::is_control` en Rust
--   (U+0000 ne peut pas figurer dans un texte PostgreSQL ; il n'est donc pas
--   listé) ;
-- - les séparateurs de ligne et de paragraphe U+2028 et U+2029, qui
--   coupent aussi la ligne.
-- Sans classe `[[:cntrl:]]` : la doc (9.7.3.2) dit que l'appartenance d'un
-- caractère non ASCII à une classe dépend de la collation.
--
-- Base de développement vérifiée le 2026-10-21 : un nom de 23 caractères,
-- sans caractère de contrôle.
ALTER TABLE structure ADD CONSTRAINT structure_nom_100_caracteres CHECK (char_length(nom) <= 100);
ALTER TABLE structure ADD CONSTRAINT structure_nom_sans_controle CHECK (nom !~ (
    '[\U00000001\U00000002\U00000003\U00000004\U00000005\U00000006\U00000007\U00000008\U00000009\U0000000A'
    '\U0000000B\U0000000C\U0000000D\U0000000E\U0000000F\U00000010\U00000011\U00000012\U00000013\U00000014'
    '\U00000015\U00000016\U00000017\U00000018\U00000019\U0000001A\U0000001B\U0000001C\U0000001D\U0000001E'
    '\U0000001F\U0000007F\U00000080\U00000081\U00000082\U00000083\U00000084\U00000085\U00000086\U00000087'
    '\U00000088\U00000089\U0000008A\U0000008B\U0000008C\U0000008D\U0000008E\U0000008F\U00000090\U00000091'
    '\U00000092\U00000093\U00000094\U00000095\U00000096\U00000097\U00000098\U00000099\U0000009A\U0000009B'
    '\U0000009C\U0000009D\U0000009E\U0000009F\U00002028\U00002029]'
));
