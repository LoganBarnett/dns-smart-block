-- Add 'expired' to classification_action so reconcile can write expiry events
-- without mixing concerns into the 'classified' action type.
ALTER TYPE classification_action ADD VALUE 'expired';
