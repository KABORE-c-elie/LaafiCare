/*
 * Point d'entrée secondaire `commun/fenetre` (angular.dev/tools/libraries/creating-libraries).
 * À part pour que la CDK (Dialog, Overlay) ne soit chargée qu'avec les pages
 * qui ouvrent une fenêtre, jamais dans le paquet initial.
 */

export * from './fenetre';
