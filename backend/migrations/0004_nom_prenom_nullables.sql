-- Conflit trouvé en écrivant AuthPatientService : la décision actée
-- (demander_otp crée une ligne `utilisateur` "en attente", téléphone seul,
-- dès qu'un numéro inconnu demande un OTP pour créer un compte) suppose que
-- nom/prenom puissent être vides à ce stade. La contrainte NOT NULL de la
-- migration 0001 partait de l'hypothèse inverse (toujours fournis à la
-- création), qui n'est plus vraie. NULL = "pas encore complété", même
-- sémantique que mot_de_passe_hash/email pour un Patient.
ALTER TABLE utilisateur
    ALTER COLUMN nom DROP NOT NULL,
    ALTER COLUMN prenom DROP NOT NULL;
