/*
 * Point d'entrée secondaire `commun/connexion` (angular.dev/tools/libraries/creating-libraries) :
 * les écrans d'accueil, de connexion et du second facteur, partagés par les
 * applications. À part du point d'entrée principal pour rester dans les pages
 * chargées à la demande, jamais dans le paquet initial.
 */

export * from './accueil';
export * from './formulaire-connexion';
export * from './second-facteur';
