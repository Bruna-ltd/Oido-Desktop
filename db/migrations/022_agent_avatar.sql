-- An agent's face when it is a picture: the id of one that comes with the app, or the bytes of one
-- the person gave (a square PNG, cropped and shrunk by the app). At most one is set; neither is
-- the initial on a colour, as every agent had before.
ALTER TABLE agents ADD COLUMN avatar_preset TEXT;
ALTER TABLE agents ADD COLUMN avatar_image BLOB;
