      *================================================================*
      * BUSDATE  - BUSINESS DATE FROM EXEC PARM='YYYYMMDD'             *
      * PERFORM 9800-GET-BUS-DATE (SEE BUSDATEP.cpy) TO VALIDATE.      *
      *================================================================*
       01  WS-PARM                     PIC X(80) VALUE SPACES.
       01  WS-BUS-DATE                 PIC 9(08) VALUE ZERO.
       01  WS-BUS-DATE-X REDEFINES WS-BUS-DATE.
           05  WS-BUS-YYYY             PIC X(04).
           05  WS-BUS-MM               PIC X(02).
           05  WS-BUS-DD               PIC X(02).
